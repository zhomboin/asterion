"""根据宿主机主进程的启动参数检查磁盘配置，并向同一实例请求重载。"""

import os
from pathlib import Path
import re
import signal
import shutil
import stat
import subprocess
import tempfile

from common import Refused, fixed_path, require, revision


def options(title):
    # Nginx 将 argv 改写为一行。保留 -g 后面包含空格的全局指令。
    parts = re.split(r"\s+-(?=[cpeg])", title)
    argv = []
    for item in parts[1:]:
        flag, value = item[0], item[1:].strip()
        require(value and " -" not in value, "nginx_arguments")
        require(flag != "p" or os.path.isabs(value), "nginx_arguments")
        argv.extend(["-" + flag, value])
    return parts[0], argv


def master(processes=None):
    masters = []
    for proc in Path("/proc").glob("[0-9]*") if processes is None else processes:
        try:
            if (proc / "comm").read_bytes().strip() != b"nginx":
                continue
            title = (proc / "cmdline").read_bytes().rstrip(b"\0").decode()
            if not title.startswith("nginx: master process "):
                continue
            uid = re.search(
                rb"^Uid:\s+\d+\s+(\d+)", (proc / "status").read_bytes(), re.M
            )
            if not uid or int(uid[1]) not in (0, os.geteuid()):
                continue
            try:
                if not (proc / "root").samefile("/"):
                    continue
            except PermissionError:
                pass
            binary, argv = options(title.removeprefix("nginx: master process "))
            try:
                binary = str((proc / "exe").resolve(strict=True))
            except PermissionError:
                if binary == "nginx":
                    binary = (
                        shutil.which("nginx", path="/usr/sbin:/usr/bin:/usr/local/sbin")
                        or ""
                    )
                require(os.path.isabs(binary), "nginx_arguments")
            require(Path(binary).name == "nginx", "nginx_arguments")
            started = (proc / "stat").read_text().rsplit(")", 1)[1].split()[19]
            masters.append((int(proc.name), started, binary, argv))
        except (ProcessLookupError, FileNotFoundError, PermissionError):
            continue
    require(masters, "nginx_missing")
    require(len(masters) == 1, "nginx_multiple")
    return masters[0]


def run(instance, flag):
    pid, _, binary, argv = instance
    if os.geteuid() == 0:
        fixed_path(binary)
        require(
            Path(binary).samefile(Path("/proc") / str(pid) / "exe"), "target_changed"
        )
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        try:
            proc = subprocess.run(
                [binary, *argv, flag],
                stdin=subprocess.DEVNULL,
                stdout=output,
                stderr=errors,
                timeout=5,
            )
        except subprocess.TimeoutExpired:
            raise Refused("timeout") from None
        if proc.returncode:
            # 仅返回位置，Nginx 的报错正文可能包含配置中的密钥或口令。
            errors.seek(0)
            location = re.search(rb" in (/[^\r\n]+:\d+)(?:\r?\n|$)", errors.read(8192))
            return None, location[1].decode(errors="replace") if location else ""
        require(output.tell() <= 16 * 1024 * 1024, "nginx_too_large")
        output.seek(0)
        return output.read(), ""


def paths(instance):
    dump, _ = run(instance, "-T")
    require(dump is not None, "nginx_check")
    found, pos = [], 0
    while pos < len(dump):
        header = re.compile(rb"# configuration file (/[^\r\n]+):\n").match(dump, pos)
        require(header and len(found) < 1024, "nginx_dump")
        name = header[1].decode()
        # 按原文件长度跨过正文，注释中的同名标记不能伪造配置来源。
        with open(
            name, "rb", opener=lambda path, flags: os.open(path, flags | os.O_NONBLOCK)
        ) as file:
            info = os.fstat(file.fileno())
            require(
                stat.S_ISREG(info.st_mode) and info.st_size <= len(dump), "nginx_dump"
            )
            body = file.read(info.st_size)
        require(dump.startswith(body + b"\n", header.end()), "nginx_dump")
        pos = header.end() + len(body) + 1
        found.append(name)
    require(found, "nginx_dump")
    return found


def selected(path):
    instance = master()
    require(path in paths(instance), "target_changed")
    return instance


def identity(instance):
    return revision(*instance) if instance else None


def reload_checked(instance, unchanged):
    output, location = run(instance, "-t")
    if output is None:
        return "Nginx 配置校验失败" + ("（" + location + "）" if location else "")
    require(
        hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"),
        "unavailable",
    )
    fd = os.pidfd_open(instance[0])
    try:
        require(master() == instance, "target_changed")
        unchanged()
        signal.pidfd_send_signal(fd, signal.SIGHUP)
    finally:
        os.close(fd)
    return None
