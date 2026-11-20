"""交互启用已有中心的配置编辑和容器操作实例。"""

import grp
import json
import os
import time
from dataclasses import dataclass
from pathlib import Path

import docker_reader
import existing
import host
import managed_targets as targets
import services
from inputs import ask

TEMPLATE = "asterion-managed@.service"
LABELS = {"configs": "服务配置", "containers": "Docker 容器操作"}
EXECUTOR = "executor.json"
VARIABLE = "ASTERION_EXECUTOR_CONFIG_FILE"


@dataclass
class Instance:
    kind: str
    path: Path
    unit: str
    override: Path
    permissions: str
    catalog: dict


def json_object(pairs):
    value = dict(pairs)
    if len(value) != len(pairs):
        raise ValueError("策略含有重复键")
    return value


def read_policy(path):
    if not existing.trusted(path, private=True):
        return {}
    if path.stat().st_size > 65536:
        raise ValueError(f"{path} 超过策略大小限制")
    data = json.loads(path.read_bytes(), object_pairs_hook=json_object)
    if not isinstance(data, dict):
        raise TypeError(f"{path} 顶层应为 JSON 对象")
    return data


def encoded(data):
    text = json.dumps(data, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
    if len(text.encode()) > 65536:
        raise ValueError("策略内容超过 64 KiB")
    return text


def publish_json(path, data):
    if not path.exists():
        host.put(path, encoded(data))
    elif read_policy(path) != data:
        host.replace_text(path, encoded(data))


def socket_name(kind):
    return f"/run/asterion-managed-{kind}/requests.sock"


def credential(unit, name):
    return f"/run/credentials/{unit}/{name}"


def loaded(unit, name, source):
    pid = services.property_value(unit, "MainPID")
    if not pid.isdecimal() or int(pid) == 0:
        return False
    path = Path("/proc") / pid / "root" / credential(unit, name).lstrip("/")
    try:
        return path.read_bytes() == source.read_bytes()
    except (FileNotFoundError, ProcessLookupError):
        return False


def confirmed_catalog(kind, previous):
    catalog = previous.copy()
    if len(catalog) > 48 or not all(
        targets.KEY.fullmatch(k) and isinstance(v, dict) for k, v in catalog.items()
    ):
        raise ValueError("已有节点策略目标格式不符")
    if catalog:
        print(f"{LABELS[kind]}已有目标：" + "、".join(sorted(catalog)))
    while len(catalog) < 48:
        if (
            catalog
            and targets.choice("1 新增授权目标，0 使用当前清单", ("0", "1"), "0") == "0"
        ):
            break
        key = ask(
            "面板目标标识（字母、数字、点、横线或下划线）",
            lambda s: targets.KEY.fullmatch(s) and s not in catalog,
        )
        entry = (
            targets.config_target()
            if kind == "configs"
            else targets.container_target(catalog)
        )
        field = "path" if kind == "configs" else "name"
        if any(old.get(field) == entry[field] for old in catalog.values()):
            raise ValueError("该文件或容器名称已绑定其他目标")
        catalog[key] = entry
    encoded(catalog)
    return catalog


def service_override(kind, catalog, path):
    first = "[Service]\n"
    if kind == "containers":
        desired = first + "SupplementaryGroups=docker\n"
    else:
        dirs = sorted({targets.config_file(v["path"]).parent for v in catalog.values()})
        first += "DynamicUser=no\nUser=root\n"
        desired = first + "".join(f"ReadWritePaths={p}\n" for p in dirs)
    previous = path.read_text() if existing.trusted(path) else desired
    if previous != desired and (
        kind != "configs"
        or not previous.startswith(first)
        or not set(previous[len(first) :].splitlines())
        <= set(desired[len(first) :].splitlines())
    ):
        raise ValueError(f"已有自定义服务权限，请核对 {path}")
    return previous, desired


def group_id(kind, *, create=False):
    name = "asterion-ops-" + kind
    try:
        group = grp.getgrnam(name)
    except KeyError:
        if not create:
            return None
        host.run("groupadd", "--system", name)
        group = grp.getgrnam(name)
    if not group.gr_gid or grp.getgrgid(group.gr_gid).gr_name != name:
        raise ValueError(f"{name} 与现有系统组冲突")
    return group.gr_gid


def prepare(kind, config, source):
    path = config / f"managed-{kind}.json"
    unit = f"asterion-managed@{kind}.service"
    override = host.UNITS / (unit + ".d/override.conf")
    existing.trusted(override.parent, directory=True)
    group_id(kind)
    catalog = confirmed_catalog(kind, read_policy(path))
    for key, spec in catalog.items():
        if kind == "configs":
            if not (
                isinstance(spec.get("path"), str)
                and targets.UNIT.fullmatch(str(spec.get("unit", "")))
                and spec.get("reload") in ("reload", "restart")
                and isinstance(spec.get("fields"), dict)
                and spec["fields"]
            ):
                raise ValueError(
                    f"服务配置目标 {key} 缺少有效的文件、Unit、动作或字段规则"
                )
        elif not targets.KEY.fullmatch(str(spec.get("name", ""))) or not spec.get(
            "engine_id"
        ):
            raise ValueError(f"容器目标 {key} 缺少有效的名称或 Engine ID")
    previous, desired = service_override(kind, catalog, override)
    services.check_unit(unit, source, {override: previous}, template_unit=TEMPLATE)
    return Instance(kind, path, unit, override, desired, catalog)


def executor_policy(path, kinds, drop):
    policy = read_policy(path)
    managed = policy.get("managed", {})
    if not isinstance(managed, dict):
        raise TypeError(f"请核对 {path} 中的 managed 策略")
    if not existing.trusted(drop) and (
        set(policy) - {"managed"} or set(managed) - set(kinds)
    ):
        raise ValueError(f"{path} 有尚未启用的其他授权，请先核对原策略")
    for kind in kinds:
        socket = socket_name(kind)
        if managed.get(kind, socket) != socket:
            raise ValueError(f"{kind} 已配置其他操作端点")
        managed[kind] = socket
    return {**policy, "managed": managed}


def check_release(release):
    deploy = release / "deploy"
    managed = deploy / "managed"
    for folder in (deploy, managed):
        if not existing.trusted(folder, directory=True):
            raise ValueError("当前发布缺少节点操作程序，请先升级中心与 Agent")
    for member in managed.rglob("*"):
        existing.trusted(member, directory=member.is_dir())
    for name in ("service", "host", "common", "containers", "console"):
        if not existing.trusted(managed / (name + ".py")):
            raise ValueError(f"当前发布缺少节点操作模块 {name}，请先升级")
    source = deploy / TEMPLATE
    if (
        not existing.trusted(source)
        or source.read_bytes() != (host.HERE.parent / TEMPLATE).read_bytes()
    ):
        raise ValueError("当前发布的节点操作服务模板不匹配，请先核对发布版本")
    return source


def configure(base, agent_base, config):
    option = targets.choice(
        "节点操作：1 服务配置编辑与重启，2 Docker 容器操作，3 两项都配置，0 跳过",
        ("0", "1", "2", "3"),
        "0",
    )
    if option == "0":
        return
    kinds = {"1": ["configs"], "2": ["containers"], "3": list(LABELS)}[option]
    if "containers" in kinds:
        status, message = docker_reader.detect()
        if status != "ready" or not os.access("/usr/bin/docker", os.X_OK):
            print(
                f"Docker 容器操作需要运行中的系统 Engine 和 /usr/bin/docker。{message}"
            )
            if (
                option != "3"
                or targets.choice("1 继续配置服务文件，0 退出", ("0", "1"), "0") == "0"
            ):
                return
            kinds = ["configs"]
    release = docker_reader.installed_release(base, agent_base, config)
    source = check_release(release)
    agent_dir = host.UNITS / (host.AGENT + ".d")
    drop = agent_dir / "40-operations.conf"
    path = config / EXECUTOR
    value = credential(host.AGENT, EXECUTOR)
    if existing.env_values(config / "agent.env").get(VARIABLE, value) != value:
        raise ValueError("Agent 已配置其他操作策略，请先核对 agent.env")
    executor = executor_policy(path, kinds, drop)
    encoded(executor)
    plans = [prepare(kind, config, source) for kind in kinds]
    for plan in plans:
        for key, spec in plan.catalog.items():
            detail = (
                f"{spec['path']} → {spec['reload']} {spec['unit']}"
                if plan.kind == "configs"
                else spec["name"]
            )
            print(f"{LABELS[plan.kind]} · {key} · {detail}")
    if (
        targets.choice(
            "应用以上授权？权限变化时将重启本机 Agent（1 应用，0 退出）",
            ("0", "1"),
            "0",
        )
        == "0"
    ):
        return
    host.unit_file(host.UNITS / TEMPLATE, source.read_text())
    groups = []
    for plan in plans:
        groups.append(group_id(plan.kind, create=True))
        pending = config / f"managed-{plan.kind}.restart-pending"
        existing.trusted(pending, private=True)
        if not plan.override.exists() or plan.override.read_text() != plan.permissions:
            host.put(pending, "\n")
        publish_json(plan.path, plan.catalog)
        if plan.override.exists() and plan.override.read_text() != plan.permissions:
            host.replace_text(plan.override, plan.permissions)
        else:
            host.unit_file(plan.override, plan.permissions)
        membership = agent_dir / f"45-managed-{plan.kind}.conf"
        host.unit_file(membership, services.agent_overrides()[membership])
    publish_json(path, executor)
    host.unit_file(drop, services.agent_overrides()[drop])
    host.run("systemctl", "daemon-reload")
    host.run("systemd-analyze", "verify", host.AGENT, *(plan.unit for plan in plans))
    for plan in plans:
        pending = config / f"managed-{plan.kind}.restart-pending"
        services.ensure_running(
            plan.unit,
            restart=pending.exists() or not loaded(plan.unit, "policy.json", plan.path),
        )
        pending.unlink(missing_ok=True)
    access = docker_reader.process_access(groups, VARIABLE, value)
    if not access or not loaded(host.AGENT, EXECUTOR, path):
        services.ensure_running(host.AGENT, restart=True)
        for _ in range(30):
            access = docker_reader.process_access(groups, VARIABLE, value)
            if access and loaded(host.AGENT, EXECUTOR, path):
                break
            time.sleep(0.2)
        else:
            raise ValueError("Agent 尚未加载节点操作授权，请检查 Agent 日志并重试")
    for plan in plans:
        with (host.HERE / "docker_probe.py").open() as probe:
            host.run(
                "/usr/bin/python3",
                "-I",
                "-B",
                "-",
                socket_name(plan.kind),
                "managed",
                *sorted(plan.catalog),
                stdin=probe,
                env={},
                cwd="/",
                **access,
            )
        print(
            f"{LABELS[plan.kind]}已启用，已核实 Agent 可访问 {len(plan.catalog)} 个授权目标。"
        )
