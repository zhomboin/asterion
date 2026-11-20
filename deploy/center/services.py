"""从数据库、持久身份和运行时凭据判断中心与本机 Agent 的初始化进度。"""

import getpass
import hashlib
import json
import re
import sqlite3
import subprocess
import time
from pathlib import Path

from existing import admin_name, local_identity, trusted
from host import AGENT, CONFIG, RUNTIME, SERVER, UNITS, put, run, template, unit_file
from inputs import password

DATABASE = Path("/var/lib/asterion/asterion.db")
IDENTITY = Path("/var/lib/asterion-agent/identity.json")
FIRST_ADMIN = Path("/run/asterion-first-admin")
ENROLL = Path("/run/asterion-self-enrollment")
ADMIN_CONF = RUNTIME / (SERVER + ".d/10-bootstrap.conf")
ENROLL_CONF = RUNTIME / (AGENT + ".d/10-self-enroll.conf")


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


def check_units(c, stage):
    for unit, known in (
        (SERVER, [ADMIN_CONF]),
        (AGENT, [ENROLL_CONF, UNITS / (AGENT + ".d/20-local.conf")]),
    ):
        unit_path = UNITS / unit
        unit_file(unit_path, (stage / "deploy" / unit).read_text(), inspect_only=True)
        fragment = run(
            "systemctl",
            "show",
            "--value",
            "-p",
            "FragmentPath",
            unit,
            stdout=subprocess.PIPE,
        ).stdout.strip()
        if fragment and Path(fragment) != unit_path:
            raise ValueError(f"{unit} 已由其他服务文件提供：{fragment}")
        overrides = run(
            "systemctl",
            "show",
            "--value",
            "-p",
            "DropInPaths",
            unit,
            stdout=subprocess.PIPE,
        ).stdout.split()
        for base in (UNITS, RUNTIME):
            directory = base / (unit + ".d")
            if trusted(directory, directory=True):
                overrides.extend(directory.glob("*.conf"))
        for item in map(Path, overrides):
            if item not in known:
                raise ValueError(
                    f"已有自定义 systemd 配置，无法确认是否为标准中心服务：{item}"
                )
        for item in known:
            if trusted(item):
                expected = (
                    template("admin.conf", ops=c["ops"])
                    if item == ADMIN_CONF
                    else template(
                        "enroll.conf" if item == ENROLL_CONF else "agent-local.conf"
                    )
                )
                unit_file(item, expected, inspect_only=True)
    trusted(FIRST_ADMIN, private=True)
    trusted(ENROLL, directory=True, private=True)
    trusted(ENROLL / "token", private=True)
    for name in ("asterion", "asterion-agent"):
        state = Path("/var/lib") / name
        if state.is_symlink() and state.resolve() != Path("/var/lib/private") / name:
            raise ValueError(f"已有状态目录指向其他位置：{state}")
    admin_name(DATABASE)
    local_identity(IDENTITY, c["port"])


def enrollment_state():
    """用本地凭证指纹关联原节点；仅凭显示名无法确定归属时由操作者确认。"""
    token = ENROLL / "token"
    receipt = CONFIG / "self-node"
    trusted(receipt, private=True)
    with sqlite3.connect(DATABASE.as_uri() + "?mode=ro", uri=True) as db:
        if IDENTITY.is_file():
            identity = json.loads(IDENTITY.read_text())
            known = db.execute(
                "SELECT n.id, n.name FROM agent_credentials a JOIN nodes n ON n.id=a.node_id "
                "WHERE a.fingerprint=? AND a.revoked_at IS NULL",
                (hashlib.sha256(identity["bearer"].encode()).digest(),),
            ).fetchone()
            if (
                known
                and known[1] == "中心服务器"
                and identity.get("node_id") in (None, known[0])
            ):
                return "identity"
            if known or identity.get("node_id"):
                raise ValueError("现有 Agent 凭证与数据库中的本机节点不匹配")
        if token.is_file():
            value = token.read_text().strip()
            if not re.fullmatch(r"[a-f0-9]{64}", value):
                raise ValueError("手动注册凭证格式不符")
            row = db.execute(
                "SELECT n.id,n.name,t.expires_at,t.consumed_at,t.revoked_at FROM node_enrollment_tokens t "
                "JOIN nodes n ON n.id=t.node_id WHERE t.fingerprint=?",
                (hashlib.sha256(value.encode()).digest(),),
            ).fetchone()
            if row:
                if row[1] != "中心服务器":
                    raise ValueError("现有注册凭证属于其他节点")
                put(receipt, row[0] + "\n")
                if row[2] > time.time() and row[3:] == (None, None):
                    return "token"
        if not receipt.exists():
            rows = db.execute(
                "SELECT id,last_seen_at FROM nodes WHERE name=?", ("中心服务器",)
            ).fetchall()
            if rows:
                if len(rows) != 1 or rows[0][1] is not None:
                    raise ValueError("已有同名节点，缺少能与其匹配的本机凭证")
                node = rows[0][0]
                if (
                    input(
                        f"发现待接入节点 {node}，确认是本机并继续注册？[y/N] "
                    ).lower()
                    != "y"
                ):
                    raise ValueError("已保留待接入节点和凭证")
                put(receipt, node + "\n")
    return "register"


def activate(c, health, monitor):
    username = admin_name(DATABASE)
    admin_drop = template("admin.conf", ops=c["ops"])
    token = ENROLL / "token"
    created = []
    running_before = {unit: active(unit) for unit in (SERVER, AGENT)}

    def temporary(path, content, mode=0o600):
        new = not path.exists()
        if path in (ADMIN_CONF, ENROLL_CONF):
            unit_file(path, content)
        else:
            put(path, content, mode)
        if new:
            created.append(path)

    try:
        for path, contents in (
            (ADMIN_CONF, admin_drop),
            (ENROLL_CONF, template("enroll.conf")),
        ):
            if path.exists():
                unit_file(path, contents)
        if username and ADMIN_CONF.exists() and not FIRST_ADMIN.exists():
            ADMIN_CONF.unlink()
        if not username:
            if not FIRST_ADMIN.exists():
                temporary(FIRST_ADMIN, password() + "\n")
            temporary(ADMIN_CONF, admin_drop, 0o644)
        run("systemctl", "daemon-reload")
        run("systemd-analyze", "verify", SERVER, AGENT)
        ensure_running(SERVER)
        health(c)
        state = enrollment_state()
        if state == "register":
            if not FIRST_ADMIN.exists():
                temporary(
                    FIRST_ADMIN,
                    getpass.getpass("现有面板管理员口令（用于登记本机 Agent）：")
                    + "\n",
                )
            monitor(
                "register",
                "--port",
                c["port"],
                "--username",
                username or c["ops"] + "-panel",
                "--receipt",
                CONFIG / "self-node",
            )
        enroll_needed = state != "identity"
        new_drop = enroll_needed and not ENROLL_CONF.exists()
        if enroll_needed:
            temporary(ENROLL_CONF, template("enroll.conf"), 0o644)
            run("systemctl", "daemon-reload")
        elif ENROLL_CONF.exists() and not token.is_file():
            ENROLL_CONF.unlink()
            run("systemctl", "daemon-reload")
        ensure_running(AGENT, restart=new_drop or state == "register")
        monitor("wait")
    except BaseException:
        # 失败时保留手动步骤的输入和已注册身份。只收回本次临时发布的文件。
        for path in reversed(created):
            path.unlink(missing_ok=True)
        if created:
            run("systemctl", "daemon-reload")
            for drop, unit in ((ENROLL_CONF, AGENT), (ADMIN_CONF, SERVER)):
                if drop in created:
                    if not running_before[unit]:
                        run("systemctl", "stop", unit)
                    elif active(unit):
                        run("systemctl", "restart", unit)
        raise

    retired = [
        unit
        for drop, unit in ((ADMIN_CONF, SERVER), (ENROLL_CONF, AGENT))
        if drop.exists()
    ]
    for path in (ADMIN_CONF, ENROLL_CONF, FIRST_ADMIN, token):
        path.unlink(missing_ok=True)
    if retired:
        run("systemctl", "daemon-reload")
        run("systemctl", "restart", *retired)
        health(c)
        monitor("wait")
