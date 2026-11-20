"""启动中心 HTTPS 服务并完成面板账号初始化。"""

import subprocess
from pathlib import Path

from existing import admin_name, trusted
from host import RUNTIME, SERVER, UNITS, put, run, template, unit_file
from inputs import password

DATABASE = Path("/var/lib/asterion/asterion.db")
FIRST_ADMIN = Path("/run/asterion-first-admin")
ADMIN_CONF = RUNTIME / (SERVER + ".d/10-bootstrap.conf")


def active(unit):
    return (
        subprocess.run(
            ["systemctl", "is-active", "--quiet", unit], check=False
        ).returncode
        == 0
    )


def ensure_running(unit, restart=False):
    if subprocess.run(
        ["systemctl", "is-enabled", "--quiet", unit], check=False
    ).returncode:
        run("systemctl", "enable", unit)
    if not active(unit):
        run("systemctl", "start", unit)
    elif restart:
        run("systemctl", "restart", unit)
    else:
        print(f"复用运行中的 {unit}。")


def property_value(unit, name):
    return run(
        "systemctl", "show", "--value", "-p", name, unit, stdout=subprocess.PIPE
    ).stdout.strip()


def check_unit(unit, source, known, *, template_unit=None):
    unit_path = UNITS / (template_unit or unit)
    unit_file(unit_path, source.read_text(), inspect_only=True)
    fragment = property_value(unit, "FragmentPath")
    if fragment and Path(fragment) != unit_path:
        raise ValueError(f"{unit} 已由其他服务文件提供：{fragment}")
    overrides = set(map(Path, property_value(unit, "DropInPaths").split()))
    for base in (UNITS, RUNTIME):
        if template_unit and (base / unit).is_file():
            raise ValueError(f"已有独立实例文件，请核对 {base / unit}")
        for name in {unit, template_unit or unit}:
            directory = base / (name + ".d")
            if trusted(directory, directory=True):
                overrides.update(directory.glob("*.conf"))
    for item in overrides:
        if item not in known:
            raise ValueError(f"已有自定义 systemd 配置，无法确认是否为标准服务：{item}")
        if trusted(item):
            unit_file(item, known[item], inspect_only=True)


def check_units(c, stage):
    check_unit(
        SERVER,
        stage / "deploy" / SERVER,
        {
            ADMIN_CONF: template("admin.conf", ops=c["ops"]),
            UNITS / (SERVER + ".d/30-ssh.conf"): template("ssh.conf"),
        },
    )
    trusted(FIRST_ADMIN, private=True)
    admin_name(DATABASE)


def activate(c, health):
    bootstrap = not admin_name(DATABASE)
    if bootstrap:
        if not trusted(FIRST_ADMIN, private=True):
            put(FIRST_ADMIN, password() + "\n")
        unit_file(ADMIN_CONF, template("admin.conf", ops=c["ops"]))
    run("systemctl", "daemon-reload")
    run("systemd-analyze", "verify", SERVER)
    ensure_running(SERVER, restart=bootstrap)
    health(c)
    if not admin_name(DATABASE):
        raise ValueError("面板账号尚未创建")
    if trusted(ADMIN_CONF):
        unit_file(ADMIN_CONF, template("admin.conf", ops=c["ops"]), inspect_only=True)
        ADMIN_CONF.unlink()
        FIRST_ADMIN.unlink(missing_ok=True)
        run("systemctl", "daemon-reload")
        run("systemctl", "restart", SERVER)
        health(c)
