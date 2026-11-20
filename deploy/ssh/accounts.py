"""读取本地账号。写入授权文件时使用目标普通账号身份。"""

import base64
import grp
import hashlib
import os
from pathlib import Path
import pwd
import re
import shlex

from common import atomic, command, fixed_path, redact, require, result


PROTECTED = set(
    "root sudo wheel adm docker lxd libvirt staff disk shadow systemd-journal kvm microk8s incus-admin podman operator daemon tty".split()
)


def memberships(user):
    return {
        g.gr_name
        for g in grp.getgrall()
        if user.pw_name in g.gr_mem and g.gr_gid != user.pw_gid
    }


def inventory():
    users = pwd.getpwall()
    groups = grp.getgrall()
    return dict(
        users=[
            dict(
                username=u.pw_name,
                uid=u.pw_uid,
                gid=u.pw_gid,
                shell=u.pw_shell,
                home=u.pw_dir,
                groups=[g.gr_name for g in groups if u.pw_name in g.gr_mem],
                system=not 1000 <= u.pw_uid < 65534,
            )
            for u in users[:512]
        ],
        truncated=len(users) > 512,
    )


def key_info(line, adding=False):
    try:
        parts = shlex.split(line.strip(), comments=False)
        keytype = re.compile(
            r"(?:ssh-(?:ed25519|rsa)|ecdsa-sha2-nistp(?:256|384|521)|sk-(?:ssh-ed25519|ecdsa-sha2-nistp256)@openssh.com)"
        )
        offset = 0 if parts and keytype.fullmatch(parts[0]) else 1
        if adding and offset or not keytype.fullmatch(parts[offset]):
            return None
        data = base64.b64decode(parts[offset + 1] + "==", validate=False)
        size = int.from_bytes(data[:4], "big")
        if (
            size != len(parts[offset])
            or data[4 : 4 + size] != parts[offset].encode()
            or len(data) < 12 + size
        ):
            return None
        fingerprint = (
            base64.b64encode(hashlib.sha256(data).digest()).decode().rstrip("=")
        )
        return dict(
            type=parts[offset],
            fingerprint="SHA256:" + fingerprint,
            comment=redact(" ".join(parts[offset + 2 :])[:256]),
        )
    except (IndexError, ValueError):
        return None


def as_owner(user):
    if os.geteuid() == 0:
        os.setgroups([])
        os.setgid(user.pw_gid)
        os.setuid(user.pw_uid)
    require(os.geteuid() == user.pw_uid, "permission_denied")


def key_file(user, change=None):
    as_owner(user)
    home = Path(user.pw_dir)
    directory = home / ".ssh"
    path = directory / "authorized_keys"
    require(fixed_path(home, directory=True).owner() == user.pw_name)
    if change and change["type"] == "add_key":
        directory.mkdir(mode=0o700, exist_ok=True)
    original = b""
    if directory.exists():
        fixed_path(directory, directory=True)
        if path.exists() or path.is_symlink():
            require(fixed_path(path).stat().st_size <= 2**20)
            original = path.read_bytes()
    lines = original.decode().splitlines(keepends=True)
    parsed = [info for line in lines if (info := key_info(line))]
    if not change:
        return dict(keys=parsed[:128], truncated=len(parsed) > 128)
    if change["type"] == "add_key":
        key = change["key"]
        info = key_info(key, True)
        require(
            info and not any(x["fingerprint"] == info["fingerprint"] for x in parsed),
            "invalid_argument",
        )
        # 由 OpenSSH 再验证真实公钥编码。
        import subprocess

        check = subprocess.run(
            ["/usr/bin/ssh-keygen", "-lf", "-"],
            input=key.encode(),
            capture_output=True,
            timeout=3,
        )
        require(check.returncode == 0, "invalid_argument")
        new = (
            original.rstrip(b"\n") + (b"\n" if original else b"") + key.encode() + b"\n"
        )
    else:
        fingerprint = change["fingerprint"]
        require(any(x["fingerprint"] == fingerprint for x in parsed), "not_found")
        new = "".join(
            line
            for line in lines
            if (key_info(line) or {}).get("fingerprint") != fingerprint
        ).encode()
    if original:
        atomic(directory / "authorized_keys.asterion-backup", original)
    atomic(path, new)
    return result("授权文件已更新")


def handle(policy, name, change=None):
    require(
        re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", name)
        and name != "root"
        and name in policy.get("usernames", [])
    )
    if change and change["type"] == "create":
        fixed_path("/home", directory=True)
        require(
            not os.path.lexists("/home/" + name)
            and name not in {u.pw_name for u in pwd.getpwall()},
            "conflict",
        )
        require(os.geteuid() == 0, "permission_denied")
        status, _ = command(
            [
                "/usr/sbin/useradd",
                "--user-group",
                "--create-home",
                "--base-dir",
                "/home",
                "--shell",
                "/bin/bash",
                "--",
                name,
            ]
        )
        return result("账号已创建" if status == 0 else "创建失败", status)
    user = pwd.getpwnam(name)
    groups = memberships(user)
    require(
        1000 <= user.pw_uid < 65534
        and 1000 <= user.pw_gid < 65534
        and user.pw_dir == "/home/" + name
        and not (groups | {grp.getgrgid(user.pw_gid).gr_name}) & PROTECTED
    )
    if not change or change["type"] in ("add_key", "remove_key"):
        return key_file(user, change)
    require(change["type"] == "set_groups" and os.geteuid() == 0, "permission_denied")
    allowed = set(policy.get("groups", []))
    selected = set(change["groups"])
    require(
        selected <= allowed
        and not allowed & PROTECTED
        and all(1000 <= grp.getgrnam(g).gr_gid < 65534 for g in allowed)
    )
    from actions import state_dir

    atomic(state_dir() / (name + ".groups.backup"), ",".join(sorted(groups)).encode())
    status, _ = command(
        [
            "/usr/sbin/usermod",
            "--groups",
            ",".join(sorted((groups - allowed) | selected)),
            "--",
            name,
        ]
    )
    return result("用户组已更新" if status == 0 else "用户组更新失败", status)
