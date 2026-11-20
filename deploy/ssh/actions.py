"""专用面板的动作和目标由中心的 SSH 配置定义。"""

import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import time

import accounts
import containers
import host
import monitor
from common import command, fixed_path, key_ok, redact, require, result


def state_dir():
    path = Path.home() / ".asterion-operations"
    path.mkdir(mode=0o700, exist_ok=True)
    require(path.stat().st_uid == os.geteuid() and path.stat().st_mode & 0o077 == 0)
    return fixed_path(path, directory=True)


def commands(policy):
    return policy.get(
        "commands", [dict(name="uptime", program="/usr/bin/uptime", timeout_seconds=5)]
    )


def catalog(policy):
    account = policy.get("accounts", {})
    return dict(
        enabled=True,
        managed=list(policy.get("managed", {})),
        systemd_units=policy.get("systemd_units", []),
        docker_targets=policy.get("docker_targets", []),
        process={"term": False, "kill": False, **policy.get("process", {})},
        accounts=dict(
            enabled=bool(account),
            usernames=account.get("usernames", []),
            groups=account.get("groups", []),
        ),
        commands=[
            dict(
                name=c["name"],
                timeout_seconds=c.get("timeout_seconds", 30),
                parameters=[
                    dict(name=k, **v) for k, v in c.get("parameters", {}).items()
                ],
            )
            for c in commands(policy)
        ],
    )


def entry(policy, group, key):
    choices = policy.get("managed", {}).get(group, {})
    require(key_ok(key) and key in choices)
    return choices[key]


def managed_query(policy, args):
    group, key = args["group"], args.get("key")
    if key is None:
        return dict(keys=sorted(policy.get("managed", {}).get(group, {})))
    spec = entry(policy, group, key)
    if group == "packages":
        return host.package_view(key, spec)
    if group == "configs":
        return host.config_view(key, spec)
    return containers.view(key, spec, state_dir())


def managed(policy, change, content):
    key, verb = change["key"], change["verb"]
    if verb == "install":
        return host.install(key, entry(policy, "packages", key), change)
    if verb == "save_reload":
        return host.save(
            key, entry(policy, "configs", key), change, content, state_dir()
        )
    return containers.apply(key, entry(policy, "containers", key), change, state_dir())


def predefined(policy, action):
    spec = next((c for c in commands(policy) if c["name"] == action["name"]), None)
    require(spec)
    rules, values = spec.get("parameters", {}), action["parameters"]
    require(rules.keys() == values.keys(), "invalid_argument")
    for k, rule in rules.items():
        v = values[k]
        kind = rule["type"]
        ok = (
            (
                kind == "integer"
                and re.fullmatch("[0-9]{1,10}", v)
                and rule["min"] <= int(v) <= rule["max"]
            )
            or (kind == "choice" and v in rule["values"])
            or (
                kind == "identifier"
                and len(v) <= rule["max_length"]
                and re.fullmatch("[A-Za-z0-9][A-Za-z0-9_.@:-]*", v)
            )
        )
        require(ok, "invalid_argument")
    argv = [str(fixed_path(spec["program"]))] + [
        a if isinstance(a, str) else values[a["parameter"]]
        for a in spec.get("args", [])
    ]
    seconds = min(120, max(1, int(spec.get("timeout_seconds", 30))))
    last, started = time.monotonic(), time.monotonic()
    output, truncated = bytearray(), False
    with subprocess.Popen(
        argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    ) as child:
        try:
            while True:
                require(time.monotonic() - started < seconds, "timeout")
                if select.select([child.stdout], [], [], 0.1)[0]:
                    data = os.read(child.stdout.fileno(), 8192)
                    if not data:
                        break
                    left = 32768 - len(output)
                    output.extend(data[:left])
                    truncated |= len(data) > left
                    if time.monotonic() - last > 0.5:
                        # 进度只包含完整行，避免拆分敏感字段。
                        complete = output.rpartition(b"\n")[0].decode(errors="replace")
                        print(
                            json.dumps(
                                dict(
                                    progress=dict(
                                        output=redact(complete), truncated=truncated
                                    )
                                )
                            ),
                            flush=True,
                        )
                        last = time.monotonic()
            status = child.wait(
                timeout=max(0.1, seconds - (time.monotonic() - started))
            )
        finally:
            if child.poll() is None:
                child.kill()
    return dict(
        exit_code=status,
        output=redact(output.decode(errors="replace")),
        truncated=truncated,
        duration_ms=0,
    )


def execute(policy, args):
    action = args["action"]
    kind = action["type"]
    if kind == "managed":
        return managed(policy, action["change"], args.get("content"))
    if kind == "command":
        return predefined(policy, action)
    if kind == "account":
        return accounts.handle(
            policy.get("accounts", {}), action["change"]["username"], action["change"]
        )
    if kind == "process":
        pid, sig = action["pid"], action["signal"]
        require(
            pid > 1
            and policy.get("process", {}).get(sig) is True
            and sig in ("term", "kill")
        )
        require(
            hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"),
            "unavailable",
        )
        fd = os.pidfd_open(pid)
        try:
            require(
                monitor.process(pid)["start_ticks"] == action["start_ticks"],
                "target_changed",
            )
            signal.pidfd_send_signal(
                fd, signal.SIGKILL if sig == "kill" else signal.SIGTERM
            )
        finally:
            os.close(fd)
        return result("信号已发送")
    verb = action["verb"]
    require(verb in ("start", "stop", "restart"))
    if kind == "systemd":
        require(action["unit"] in policy.get("systemd_units", []))
        status, _ = command(
            ["/usr/bin/systemctl", "--no-ask-password", verb, "--", action["unit"]]
        )
    else:
        require(
            kind == "docker"
            and any(
                t["engine_id"] == action["engine_id"]
                and action["container_id"] in t["container_ids"]
                for t in policy.get("docker_targets", [])
            )
        )
        require(
            containers.checked("info", "--format", "{{.ID}}") == action["engine_id"],
            "target_changed",
        )
        status, _ = containers.docker("container", verb, "--", action["container_id"])
    return result(
        "操作完成" if status == 0 else "命令执行失败，请检查 SSH 账号权限和目标状态",
        status,
    )
