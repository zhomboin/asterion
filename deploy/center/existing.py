"""核对手动安装留下的文件、服务和发布内容。"""

import filecmp
import grp
import json
import os
import pwd
import re
import shutil
import sqlite3
import stat
import subprocess
import tempfile
from pathlib import Path

from inputs import domain_ok, ipv4


def trusted(path, directory=False, private=False):
    """已知系统路径须归安装管理员所有，且不可由其他用户替换。"""
    if not os.path.lexists(path):
        return False
    info = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if (
        not kind(info.st_mode)
        or info.st_uid != os.geteuid()
        or info.st_mode & (0o077 if private else 0o022)
    ):
        raise ValueError(f"已有路径的类型、属主或权限不符：{path}")
    return True


def env_values(path):
    values = {}
    if trusted(path, private=True):
        for line in path.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            key, sep, value = line.partition("=")
            if not sep or key in values or not re.fullmatch(r"[A-Za-z_]\w*", key):
                raise ValueError(f"无法核对配置 {path}，请检查变量名、重复项和赋值格式")
            values[key] = value
    return values


def ensure_env(path, expected, put):
    values = env_values(path)
    missing = []
    for key, value in (line.split("=", 1) for line in expected.splitlines()):
        if key in values and values[key] != value:
            raise ValueError(f"{path} 中 {key} 与目标配置不一致")
        if key not in values:
            missing.append(f"{key}={value}\n")
    if not missing:
        return
    if not path.exists():
        put(path, "".join(missing))
        return
    # 保留已有注释、变量和权限，通过同目录重命名一次补全。
    with tempfile.NamedTemporaryFile(dir=path.parent, mode="w", delete=False) as out:
        tmp = Path(out.name)
        try:
            out.write(path.read_text().rstrip("\n") + "\n" + "".join(missing))
            out.flush()
            os.fchmod(out.fileno(), stat.S_IMODE(path.stat().st_mode))
            os.replace(tmp, path)
        finally:
            tmp.unlink(missing_ok=True)


def hints(config, sites, sudoers):
    result = {}
    env = env_values(config / "server.env")
    fields = [
        (
            "port",
            "ASTERION_LISTEN_ADDR",
            r"127\.0\.0\.1:([1-9][0-9]{3,4})",
            lambda s: int(s) <= 65535,
        ),
        ("domain", "ASTERION_SSH_DEPLOY_CENTER_URL", r"https://([^/]+)/?", domain_ok),
        ("node", "ASTERION_SSH_DEPLOY_NETWORKS", r"([^/]+)/32", ipv4),
    ]
    for name, key, pattern, valid in fields:
        if key in env:
            match = re.fullmatch(pattern, env[key])
            if not match or not valid(match[1]):
                raise ValueError(f"现有 server.env 中 {key} 超出标准中心配置范围")
            result[name] = match[1]
    # 只从引用本中心专用 CA 的站点推断，已有其他站点不参与选择。
    candidates = []
    for site in sites.glob("*.conf"):
        if site.is_symlink() or not site.is_file():
            continue
        data = site.read_text()
        if "proxy_ssl_trusted_certificate /etc/asterion/deploy-ca.crt;" in data:
            candidates.append(data)
    if len(candidates) == 1:
        data = candidates[0]
        domain = re.search(r"^\s*server_name\s+([\w.-]+);", data, re.MULTILINE)
        if (
            domain
            and domain_ok(domain[1])
            and result.get("domain", domain[1]) == domain[1]
        ):
            result["domain"] = domain[1]
            allowed = re.findall(r"^\s*allow\s+([\d.]+);", data, re.MULTILINE)
            if allowed and allowed[0] == "127.0.0.1" and all(map(ipv4, allowed)):
                if len(allowed) > 1:
                    result["admin_ip"] = allowed[1]
                if len(allowed) > 2:
                    result["agent"] = allowed[2]
    names = [p.name[len("asterion-") :] for p in sudoers.glob("asterion-*")]
    if len(names) == 1:
        result["ops"] = names[0]
    return result


def login_account(name):
    try:
        account = pwd.getpwnam(name)
    except KeyError:
        try:
            grp.getgrnam(name)
        except KeyError:
            return None
        raise ValueError(f"只有同名组，无法确认运维账号 {name}")
    home = Path(account.pw_dir)
    if (
        not 1000 <= account.pw_uid < 65534
        or not home.is_absolute()
        or account.pw_shell.rsplit("/", 1)[-1] in {"false", "nologin", ""}
    ):
        raise ValueError(f"{name} 不是可复用的普通登录账号")
    if home.is_symlink() or not home.is_dir() or home.stat().st_uid != account.pw_uid:
        raise ValueError(f"运维账号 {name} 的家目录属主或类型不符")
    return account


def authorized_keys(account):
    path = Path(account.pw_dir) / ".ssh/authorized_keys"
    # 使用账号自己的权限检查和使用 SSH 目录。
    return (
        subprocess.run(
            [
                "runuser",
                "-u",
                account.pw_name,
                "--",
                "sh",
                "-c",
                'test -f "$1" && test -r "$1" && test -s "$1"',
                "sh",
                path,
            ],
            check=False,
        ).returncode
        == 0
    )


def check_link(path, target):
    if os.path.lexists(path) and (
        not path.is_symlink() or path.resolve() != target.resolve()
    ):
        raise ValueError(f"{path} 与所选发布不符，请核对发布包版本")


def reconcile_release(stage, release, digest, put):
    """先检查整棵已解包的目录，再补入缺少的成员。"""
    trusted(release, directory=True)
    marker = release / ".sha256"
    if trusted(marker) and marker.read_text().strip() != digest:
        raise ValueError("已有发布目录与本次归档不同")
    for path in release.rglob("*"):
        if path == marker:
            continue
        source = stage / path.relative_to(release)
        if not source.exists():
            raise ValueError(f"已有归档外的发布文件：{path}")
        trusted(path, directory=source.is_dir())
        if source.is_file() and (
            not filecmp.cmp(path, source, shallow=False)
            or (source.stat().st_mode ^ path.stat().st_mode) & 0o111
        ):
            raise ValueError(f"已有发布文件与所选归档不一致：{path}")
    for source in stage.rglob("*"):
        target = release / source.relative_to(stage)
        if source.is_dir():
            target.mkdir(mode=0o755, exist_ok=True, parents=True)
        elif not target.exists():
            target.parent.mkdir(exist_ok=True, parents=True)
            with (
                tempfile.NamedTemporaryFile(dir=target.parent) as out,
                source.open("rb") as stream,
            ):
                shutil.copyfileobj(stream, out)
                out.flush()
                os.fchmod(out.fileno(), stat.S_IMODE(source.stat().st_mode))
                os.link(out.name, target)
    put(marker, digest + "\n")


def admin_name(database):
    if not database.exists():
        return None
    if not stat.S_ISREG(database.lstat().st_mode):
        raise ValueError(f"数据库文件类型不符：{database}")
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as conn:
        tables = {
            r[0]
            for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
        }
        if tables and "_sqlx_migrations" not in tables:
            raise ValueError("已有 SQLite 文件与中心数据库不符")
        if "panel_admin" in tables:
            row = conn.execute("SELECT username FROM panel_admin WHERE id=1").fetchone()
            return row[0] if row else None
    return None


def local_identity(path, port):
    if not os.path.lexists(path):
        return None
    if not stat.S_ISREG(path.lstat().st_mode) or path.stat().st_mode & 0o077:
        raise ValueError(f"Agent 身份文件类型或权限不符：{path}")
    identity = json.loads(path.read_text())
    if identity.get(
        "endpoint"
    ) != f"wss://127.0.0.1:{port}/api/v1/agent/connect" or not re.fullmatch(
        r"[a-f0-9]{64}", identity.get("bearer", "")
    ):
        raise ValueError("已有 Agent 身份与目标中心不符")
    node = identity.get("node_id")
    if node is not None and not re.fullmatch(r"[a-f0-9]{32}", node):
        raise ValueError("已有 Agent 节点编号无效")
    return node
