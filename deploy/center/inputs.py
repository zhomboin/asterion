"""首次安装的交互参数和发布包校验。"""

import getpass
import hashlib
import ipaddress
import re
import shutil
import tarfile
from pathlib import Path, PurePosixPath


def ask(label, valid, default="", secret=False):
    prompt = f"{label} [{default}]：" if default else f"{label}："
    while True:
        value = getpass.getpass(prompt) if secret else input(prompt).strip()
        value = value or default
        if valid(value):
            return value
        print("格式有误，请重新输入。")


def domain_ok(value):
    labels = value.split(".")
    return (
        len(value) <= 253
        and len(labels) >= 2
        and not labels[-1].isdigit()
        and all(
            re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", s) for s in labels
        )
    )


def ipv4(value):
    try:
        ipaddress.IPv4Address(value)
    except ValueError:
        return False
    return True


def settings(known=None):
    known = known or {}
    domain = known.get("domain") or ask("面板域名（已设 DNS A 记录）", domain_ok)
    port = known.get("port") or ask(
        "中心回环监听端口",
        lambda s: re.fullmatch(r"[1-9][0-9]{4}", s) and int(s) <= 65535,
        "18080",
    )
    ops = ask(
        "面板管理员名称前缀",
        lambda s: re.fullmatch(r"[a-z_][a-z0-9_-]{0,30}", s),
        known.get("ops", ""),
    )
    return {
        "domain": domain,
        "port": port,
        "ops": ops,
    }


def password():
    def valid(s):
        return (
            12 <= len(s.encode()) <= 128
            and s.strip()
            and not any(c in s for c in "\r\n\0")
        )

    while True:
        value = ask("面板管理员口令（12 至 128 个 UTF-8 字节）", valid, secret=True)
        if getpass.getpass("确认面板口令：") == value:
            return value
        print("两次输入不同。")


def release_info(archive):
    match = re.fullmatch(
        r"asterion-([A-Za-z0-9][A-Za-z0-9._-]{0,95})\.tar\.gz", archive.name
    )
    if not match:
        raise ValueError("归档名称应为 asterion-<构建编号>.tar.gz")
    line = Path(str(archive) + ".sha256").read_text().rstrip("\n")
    digest = re.fullmatch(r"([a-fA-F0-9]{64}) [ *]" + re.escape(archive.name), line)
    if not digest:
        raise ValueError("校验文件必须只包含该归档的 SHA-256 和文件名")
    return match[1], digest[1].lower()


def unpack(archive, digest, stage):
    # 校验和解包使用同一份私有快照，目标始终位于新建的暂存目录内。
    copy = stage.parent / "archive.tar.gz"
    shutil.copyfile(archive, copy)
    with copy.open("rb") as stream:
        checksum = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(block)
        if checksum.hexdigest() != digest:
            raise ValueError("归档 SHA-256 校验失败")
    with tarfile.open(copy, "r:gz") as bundle:
        for item in bundle:
            path = PurePosixPath(item.name)
            if (
                not path.parts
                or path.parts[0] not in {"bin", "deploy", "web"}
                or ".." in path.parts
            ):
                raise ValueError("归档含非发布目录成员")
            target = stage / path
            if item.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif item.isreg():
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.extractfile(item) as src, target.open("xb") as dst:
                    shutil.copyfileobj(src, dst)
                target.chmod(0o755 if path.parts[0] == "bin" else 0o644)
            else:
                raise ValueError("归档含链接或特殊文件")
    required = (
        "bin/asterion-server",
        "web/index.html",
        "deploy/asterion-server.service",
    )
    if not all(
        (stage / name).is_file() and (stage / name).stat().st_size for name in required
    ):
        raise ValueError("发布包缺少中心或面板所需的文件")
