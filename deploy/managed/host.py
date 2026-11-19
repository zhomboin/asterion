import json
import re
import tempfile

from common import atomic, command, digest, fixed_path, require, result, revision


def package_view(key, spec):
    package = spec["package"]
    require(re.fullmatch(r"[a-z0-9][a-z0-9+.-]{1,126}", package) and not package.endswith(("+", "-")))
    status, value = command(["/usr/bin/dpkg-query", "-W", "-f=${Status}", "--", package], capture=True)
    require(status in (0, 1), "execution_failed")
    return {"key": key, "package": package, "installed": status == 0 and value == "install ok installed",
            "revision": revision(key, spec), "command": " ".join(install_command(package))}


def install_command(package):
    return ["/usr/bin/apt-get", "--assume-yes", "--no-remove", "--no-install-recommends",
            "-o", "DPkg::Lock::Timeout=30", "install", "--", package]


def install(key, spec, action):
    current = package_view(key, spec)
    require(action["revision"] == current["revision"], "target_changed")
    if current["installed"]:
        return result("软件已安装")
    available, records = command(["/usr/bin/apt-cache", "show", "--no-all-versions", "--",
                                  current["package"]], capture=True)
    if available or ("Package: " + current["package"]) not in records.splitlines():
        return result("已配置的软件源中没有这个确切的软件包", 1)
    status, _ = command(install_command(current["package"]), timeout=870)
    if status or not package_view(key, spec)["installed"]:
        return result("安装未完成，请检查节点的软件源、磁盘空间及包管理器锁，再查询安装状态。", 1)
    return result("已核实安装状态")


def read_config(spec):
    path = fixed_path(spec["path"])
    require(path.stat().st_size <= 8192 and path.stat().st_mode & 0o7000 == 0)
    data = path.read_bytes()
    require(len(data) <= 8192 and b"\x00" not in data)
    require(re.fullmatch(r"[a-zA-Z0-9_][a-zA-Z0-9_.@-]{0,245}\.service", spec["unit"]))
    require(spec["reload"] in ("reload", "restart") and spec["fields"])
    return path, data


def config_view(key, spec):
    path, data = read_config(spec)
    return {"key": key, "path": str(path), "unit": spec["unit"], "reload": spec["reload"],
            "revision": revision(spec, digest(data)), "content": data.decode("utf-8")}


def decode_config(data):
    def unique(pairs):
        value = dict(pairs)
        require(len(pairs) == len(value), "invalid_argument")
        return value
    value = json.loads(data, object_pairs_hook=unique,
                       parse_constant=lambda _: require(False, "invalid_argument"))
    require(isinstance(value, dict), "invalid_argument")
    return value


def validate_config(original, candidate, rules):
    before = decode_config(original)
    after = decode_config(candidate)
    require(before.keys() == after.keys(), "invalid_argument")
    for name, value in after.items():
        rule = rules.get(name)
        if rule is None:
            require(revision(value) == revision(before[name]), "invalid_argument")
        elif "choices" in rule:
            require(any(type(item) is type(value) and item == value for item in rule["choices"]), "invalid_argument")
        else:
            require(type(value) is int and rule["min"] <= value <= rule["max"], "invalid_argument")


def validate_candidate(path, spec, previous, data):
    if "template" in spec:
        parts = re.split(r"(\{\{[A-Za-z][A-Za-z0-9_]*\}\})", spec["template"])
        pattern = []
        for part in parts:
            if not re.fullmatch(r"\{\{[A-Za-z][A-Za-z0-9_]*\}\}", part):
                pattern.append(re.escape(part))
                continue
            choices = spec["fields"][part[2:-2]]["choices"]
            require(0 < len(choices) <= 48 and all(isinstance(value, str) and len(value) <= 256 for value in choices))
            pattern.append("(?:" + "|".join(map(re.escape, choices)) + ")")
        require(re.fullmatch("".join(pattern), data.decode()), "invalid_argument")
    else:
        validate_config(previous, data, spec["fields"])
    if "validate" in spec:
        validator = spec["validate"]
        require(1 < len(validator) <= 16 and validator.count("{candidate}") == 1)
        fixed_path(validator[0])
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".asterion-check-") as candidate:
            candidate.write(data)
            candidate.flush()
            require(command([candidate.name if arg == "{candidate}" else arg for arg in validator], 8)[0] == 0,
                    "invalid_argument")


def save(key, spec, action, content, state):
    path, previous = read_config(spec)
    require(action["revision"] == revision(spec, digest(previous)), "target_changed")
    require(isinstance(content, str), "invalid_argument")
    data = content.encode("utf-8")
    require(len(data) <= 8192 and digest(data) == action["digest"], "invalid_argument")
    validate_candidate(path, spec, previous, data)
    if previous == data:
        return result("配置没有变化")
    backup = state / (key + ".backup")
    atomic(backup, previous)
    require(fixed_path(spec["path"]).read_bytes() == previous, "target_changed")
    atomic(path, data, path)
    control = ["/usr/bin/systemctl", "--system", "--no-ask-password", spec["reload"], "--", spec["unit"]]

    def activate():
        try:
            return command(control, 12)[0] == 0 and command(
                ["/usr/bin/systemctl", "is-active", "--quiet", "--", spec["unit"]], 3)[0] == 0
        except Exception:
            return False

    if activate():
        return result("配置已保存，已核实服务活动状态")
    if fixed_path(spec["path"]).read_bytes() != data:
        return result("服务应用失败；文件已被外部修改，未覆盖。请按节点备份恢复。", 1)
    atomic(path, previous, path)
    if activate():
        return result("服务应用失败；已恢复原配置并核实服务活动状态", 1)
    return result("原配置已恢复，但服务未确认恢复；请在节点检查服务。", 1)
