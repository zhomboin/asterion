import json
import re
import secrets
from pathlib import Path

from common import Refused, atomic, command, fixed_path, key_ok, require, result, revision


def docker(*args):
    return command(["/usr/bin/docker", "--config", "/var/empty", "--host",
                    "unix:///var/run/docker.sock", *args], 12, capture=True)


def checked(*args):
    status, text = docker(*args)
    require(status == 0, "execution_failed")
    return text


def inspect(container, field):
    return checked("container", "inspect", "--format", field, "--", container)


def console_target(key, spec, requested, state):
    require(spec.get("terminal") is True and spec["engine_id"] == requested["engine"])
    target = binding(key, spec, state)
    require(target and target["id"] == requested["id"], "target_changed")
    container = target["id"]
    details = inspect(container, "{{.State.Running}}\n{{json .HostConfig}}\n{{json .Mounts}}").splitlines()
    require(len(details) == 3 and details[0] == "true")
    config = json.loads(details[1])
    require(not any(config.get(name) for name in ("Privileged", "CapAdd", "Devices", "DeviceRequests")))
    require(not any(config.get(mode) == "host" for mode in ("NetworkMode", "PidMode", "IpcMode", "UTSMode")))
    for mount in json.loads(details[2]):
        if mount["Type"] == "bind":
            require(not mount["RW"])
            fixed_path(mount["Source"])
    return container


def binding(key, spec, state):
    require(key_ok(spec["name"]))
    engine = checked("info", "--format", "{{.ID}}")
    require(engine and engine == spec["engine_id"], "target_changed")
    record = state / (key + ".json")
    if "existing_id" in spec:
        container = spec["existing_id"]
        require(re.fullmatch(r"[0-9a-f]{64}", container))
        require(inspect(container, "{{.Id}} {{.Name}}") == container + " /" + spec["name"], "target_changed")
        return {"id": container, "engine": engine, "profile": None}
    if not record.exists():
        return None
    target = json.loads(record.read_bytes())
    require(re.fullmatch(r"[0-9a-f]{64}", target["id"]) and target["engine"] == engine, "target_changed")
    identity = inspect(target["id"], '{{.Id}} {{index .Config.Labels "io.asterion.managed"}} {{.Name}}')
    require(identity == f'{target["id"]} {key} /{spec["name"]}', "target_changed")
    return target


def view(key, spec, state):
    target = binding(key, spec, state)
    running = bool(target and inspect(target["id"], "{{.State.Running}}") == "true")
    profiles = spec.get("profiles", {})
    require((bool(profiles) != ("existing_id" in spec)) and len(profiles) <= 12 and all(key_ok(name) for name in profiles))
    public = {name: {field: value for field, value in profile.items() if field in
                      ("image", "restart", "ports", "mounts", "restart_choices", "port_choices", "mount_choices")}
              for name, profile in profiles.items()}
    return {"key": key, "name": spec["name"], "engine": spec["engine_id"], "target": target,
            "running": running, "profiles": public, "terminal": spec.get("terminal") is True,
            "revision": revision(key, spec, target, running)}


def creation(key, profile, name):
    image = profile["image"]
    require(re.fullmatch(r"[a-z0-9][a-z0-9./:_-]{0,180}@sha256:[0-9a-f]{64}", image))
    require(key_ok(name) and key_ok(key))
    policy = profile["restart"]
    require(policy in ("no", "unless-stopped", "on-failure", "always"))
    args = ["container", "create", "--pull=never", "--name", name, "--label",
            "io.asterion.managed=" + key, "--cap-drop=ALL", "--security-opt=no-new-privileges",
            "--restart", policy]
    for port in profile.get("ports", []):
        require(len(port) == 2 and all(type(number) is int and 1 <= number <= 65535 for number in port))
        args += ["--publish", f"127.0.0.1:{port[0]}:{port[1]}/tcp"]
    seen = set()
    for mount in profile.get("mounts", []):
        require(set(mount) in ({"volume", "destination"}, {"source", "destination"}))
        dest = mount["destination"]
        require(re.fullmatch(r"(/[a-zA-Z0-9_.-]+)+", dest) and Path(dest).as_posix() == dest
                and ".." not in Path(dest).parts and dest not in seen)
        seen.add(dest)
        if "volume" in mount:
            require(key_ok(mount["volume"]))
            checked("volume", "inspect", "--format", "{{.Name}}", "--", mount["volume"])
            source = "type=volume,src=" + mount["volume"]
        else:
            path = fixed_path(mount["source"])
            require("," not in str(path))
            source = "type=bind,readonly,src=" + str(path)
        args += ["--mount", source + ",dst=" + dest]
    declared = json.loads(checked("image", "inspect", "--format", "{{json .Config.Volumes}}", "--", image))
    require(set(declared or {}) <= seen)
    return [*args, "--", image]


def configured(profile, settings):
    if settings is None:
        return profile
    require(set(settings) == {"restart", "ports", "mounts"})
    restart, ports, mounts = settings["restart"], settings["ports"], settings["mounts"]
    require(restart in profile.get("restart_choices", [profile["restart"]]))
    require(isinstance(ports, list) and len(ports) <= 16)
    require(isinstance(mounts, list) and len(mounts) <= 16 and all(key_ok(name) for name in mounts))
    require(len(set(mounts)) == len(mounts))
    choices = profile.get("mount_choices", {})
    require(all(name in choices for name in mounts))
    require(all(port in profile.get("port_choices", []) for port in ports))
    effective = {**profile, "restart": restart, "ports": profile.get("ports", []) + ports,
                 "mounts": profile.get("mounts", []) + [choices[name] for name in mounts]}
    require(len({pair[0] for pair in effective["ports"]}) == len(effective["ports"]))
    return effective


def apply(key, spec, action, state):
    current = view(key, spec, state)
    require(current["revision"] == action["revision"], "target_changed")
    target = current["target"]
    verb = action["verb"]
    if verb in ("start", "stop", "restart"):
        require(target)
        checked("container", verb, "--", target["id"])
        require((inspect(target["id"], "{{.State.Running}}") == "true") == (verb != "stop"), "execution_failed")
        return result("已核实容器运行状态")
    require((verb == "create" and not target) or (verb == "recreate" and target))
    require("existing_id" not in spec)
    profile = action["profile"]
    require(profile in spec["profiles"])
    name = spec["name"]
    alternate = "asterion-" + secrets.token_hex(12)
    settings = action.get("settings")
    effective = configured(spec["profiles"][profile], settings)
    candidate = checked(*creation(key, effective, alternate if target else name))
    require(re.fullmatch(r"[0-9a-f]{64}", candidate), "execution_failed")
    renamed = False
    try:
        if target:
            if current["running"]:
                checked("container", "stop", "--time", "5", "--", target["id"])
            checked("container", "rename", "--", target["id"], alternate + "-old")
            renamed = True
            checked("container", "rename", "--", candidate, name)
            if current["running"]:
                checked("container", "start", "--", candidate)
        require(inspect(candidate, "{{.Id}} {{.Name}} {{.State.Running}}") ==
                f'{candidate} /{name} {str(current["running"]).lower()}', "target_changed")
        atomic(state / (key + ".json"), json.dumps(
            {"id": candidate, "engine": current["engine"], "profile": profile, "settings": settings}).encode())
    except (Refused, OSError):
        try:
            checked("container", "rm", "--force", "--", candidate)
            if renamed:
                checked("container", "rename", "--", target["id"], name)
            if target and current["running"]:
                checked("container", "start", "--", target["id"])
        except (Refused, OSError):
            return result("容器回退未完成，请核对旧容器与候选容器：" +
                          (target["id"] if target else "无") + " / " + candidate, 1)
        return result("创建未完成；已撤销候选容器，未更改服务绑定", 1)
    if target and docker("container", "rm", "--", target["id"])[0]:
        return result("新容器已绑定，旧容器未清理。请在节点核查。", 1)
    return result("已核实并绑定容器：" + candidate)
