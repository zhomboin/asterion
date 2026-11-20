"""从管理员选定的配置文件、容器和本地镜像生成节点策略。"""

import json
import re
import subprocess
from pathlib import Path

import existing
import host
import services
from inputs import ask

KEY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}")
UNIT = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.@-]{0,245}\.service")
IMAGE = re.compile(r"[a-z0-9][a-z0-9./:_-]{0,180}@sha256:[a-f0-9]{64}")
SAFE_PATH = re.compile(r"(/[A-Za-z0-9_.-]+)+")


def choice(label, values, default=""):
    return ask(label, lambda value: value in values, default)


def pick(label, values):
    if not values:
        raise ValueError(f"{label}：没有可选择的项目")
    for index, value in enumerate(values, 1):
        print(f"  {index}) {value}")
    number = ask(
        label + "编号",
        lambda s: s.isascii() and s.isdecimal() and 1 <= int(s) <= len(values),
    )
    return values[int(number) - 1]


def target_file(value):
    path = Path(value)
    if not SAFE_PATH.fullmatch(str(path)) or path.resolve(strict=True) != path:
        raise ValueError("请选择使用普通目录名、无软链接的绝对路径")
    for parent in path.parents:
        existing.trusted(parent, directory=True)
    if (
        not existing.trusted(path)
        or path.stat().st_nlink != 1
        or path.stat().st_mode & 0o7111
    ):
        raise ValueError(f"请选择普通的非可执行文件：{path}")
    return path


def config_file(value):
    path = target_file(value)
    if len(path.parent.parts) < 3 or path.parent in map(
        Path,
        ("/usr/bin", "/usr/sbin", "/usr/lib", "/usr/local", "/var/lib", "/var/log"),
    ):
        raise ValueError("请选择应用独立配置目录中的文件")
    if path.stat().st_size > 8192:
        raise ValueError("配置文件超过 8192 字节")
    return path


def field_rule(current):
    if type(current) is int:
        low = ask(
            "允许的最小整数",
            lambda s: re.fullmatch("-?[0-9]{1,18}", s) and int(s) <= current,
        )
        high = ask(
            "允许的最大整数",
            lambda s: re.fullmatch("-?[0-9]{1,18}", s) and int(s) >= current,
        )
        return {"min": int(low), "max": int(high)}
    if type(current) is bool:
        return {"choices": [False, True]}
    other = ask(
        "允许改为哪些字符串（多个用英文逗号分隔）",
        lambda s: any(x.strip() != current for x in s.split(",") if x.strip()),
    )
    return {
        "choices": list(
            dict.fromkeys(
                [current] + [x.strip() for x in other.split(",") if x.strip()]
            )
        )
    }


def json_fields(data):
    rules = {}
    keys = [key for key, value in data.items() if isinstance(value, (str, int, bool))]
    while True:
        key = pick("选择可编辑的 JSON 字段", keys)
        rules[key] = field_rule(data[key])
        keys.remove(key)
        if not keys or choice("1 再授权一个字段，0 完成", ("1", "0"), "0") == "0":
            return {"fields": rules}


def fields_for(content):
    try:
        data = json.loads(content)
    except json.JSONDecodeError:
        data = None
    if isinstance(data, dict):
        return json_fields(data)
    if re.search(r"\{\{[A-Za-z][A-Za-z0-9_]*\}\}", content):
        raise ValueError("此文本已有模板标记，请使用已有的专用配置策略")
    old = ask(
        "输入文件中唯一出现、允许编辑的原文本",
        lambda s: bool(s) and len(s) <= 256 and content.count(s) == 1,
    )
    other = ask(
        "允许替换成的文本（多个用英文逗号分隔）",
        lambda s: (
            0 < len(s.split(",")) < 48
            and all(0 < len(v.strip()) <= 256 for v in s.split(","))
            and any(v.strip() != old for v in s.split(","))
        ),
    )
    return {
        "fields": {"value": {"choices": [old, *[v.strip() for v in other.split(",")]]}},
        "template": content.replace(old, "{{value}}"),
    }


def config_target():
    path = config_file(ask("可完整展示给面板管理员的无密钥应用配置文件绝对路径", bool))
    unit = ask("保存后应用的 systemd 服务（含 .service）", UNIT.fullmatch)
    if services.property_value(unit, "LoadState") != "loaded":
        raise ValueError(f"找不到已安装的服务 {unit}")
    action = choice(
        "保存后执行 restart 重启或 reload 重载", ("restart", "reload"), "restart"
    )
    return {
        "path": str(path),
        "unit": unit,
        "reload": action,
        **fields_for(path.read_text()),
    }


def docker(*args):
    return host.run(
        "/usr/bin/docker",
        "--config",
        "/var/empty",
        "--host",
        "unix:///var/run/docker.sock",
        *args,
        stdout=subprocess.PIPE,
        timeout=15,
    ).stdout.strip()


def destination(value):
    path = Path(value)
    return SAFE_PATH.fullmatch(value) and str(path) == value and ".." not in path.parts


def ports(value):
    result = []
    for item in value.split(",") if value else []:
        match = re.fullmatch(r"([0-9]{1,5}):([0-9]{1,5})", item.strip())
        if not match or any(not 1 <= int(n) <= 65535 for n in match.groups()):
            raise ValueError("端口应为 1 至 65535 的 主机端口:容器端口")
        result.append(list(map(int, match.groups())))
    if len(result) > 16 or len({tuple(pair) for pair in result}) != len(result):
        raise ValueError("请提供最多 16 个不同的端口映射")
    return result


def container_target(catalog):
    engine = docker("info", "--format", "{{.ID}}")
    if not engine:
        raise ValueError("无法读取本机 Docker Engine ID")
    mode = choice("1 授权已有容器启停，2 定义可创建和重建的新容器", ("1", "2"), "1")
    listing = docker(
        "ps", "-a", "--no-trunc", "--format", "{{.Names}} {{.ID}}"
    ).splitlines()
    known = dict(line.split() for line in listing)
    if mode == "1":
        name = pick(
            "选择已有容器", sorted(name for name in known if KEY.fullmatch(name))
        )
        if not re.fullmatch("[0-9a-f]{64}", known[name]):
            raise ValueError("未取得完整容器 ID")
        return {"engine_id": engine, "name": name, "existing_id": known[name]}
    name = ask(
        "新容器名称",
        lambda s: (
            KEY.fullmatch(s)
            and s not in known
            and all(entry.get("name") != s for entry in catalog.values())
        ),
    )
    images = docker(
        "image", "ls", "--digests", "--format", "{{.Repository}}@{{.Digest}}"
    ).splitlines()
    image = pick("选择已有摘要的本地镜像", sorted(set(filter(IMAGE.fullmatch, images))))
    mappings = ports(
        input("允许面板选择的本机 TCP 映射（如 8080:80,8081:80，回车跳过）：").strip()
    )
    volumes = docker("volume", "ls", "--format", "{{.Name}}").splitlines()
    declared = (
        json.loads(
            docker(
                "image", "inspect", "--format", "{{json .Config.Volumes}}", "--", image
            )
        )
        or {}
    )
    fixed = []
    optional = {}
    used = set(declared)
    for dest in sorted(declared):
        if not destination(dest):
            raise ValueError("镜像声明的卷路径不适合作为受管目标")
        fixed.append(
            {
                "destination": dest,
                "volume": pick(
                    f"为 {dest} 选择必需的命名卷", list(filter(KEY.fullmatch, volumes))
                ),
            }
        )
    while len(optional) < 16:
        kind = choice(
            "可选挂载：1 只读文件，2 已有数据卷，0 完成", ("1", "2", "0"), "0"
        )
        if kind == "0":
            break
        if kind == "1":
            mount = {
                "source": str(
                    target_file(ask("允许容器只读访问的非敏感文件绝对路径", bool))
                )
            }
        else:
            mount = {
                "volume": pick("选择可用命名卷", list(filter(KEY.fullmatch, volumes)))
            }
        dest = ask("容器内挂载路径", lambda s: destination(s) and s not in used)
        used.add(dest)
        optional[f"mount-{len(optional) + 1}"] = {**mount, "destination": dest}
    return {
        "name": name,
        "engine_id": engine,
        "profiles": {
            "standard": {
                "image": image,
                "restart": "unless-stopped",
                "restart_choices": ["no", "unless-stopped", "on-failure", "always"],
                "mounts": fixed,
                "mount_choices": optional,
                "port_choices": mappings,
            }
        },
    }
