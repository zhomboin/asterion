import os
from contextlib import contextmanager
from pathlib import Path
import stat

import nginx_config
from common import Refused, atomic, digest, require, result, revision

LIMIT = 64 * 1024


def valid_text(data):
    require(len(data) <= LIMIT, "file_too_large")
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise Refused("file_not_text") from None
    require(
        all(c >= " " or c in "\n\r\t" for c in text) and "\x7f" not in text,
        "file_not_text",
    )
    return text


def stamp(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_ctime_ns


@contextmanager
def opened(name):
    require(
        isinstance(name, str)
        and os.path.isabs(name)
        and len(name.encode()) <= 1024
        and all(c.isprintable() for c in name),
        "invalid_argument",
    )
    path = Path(name).resolve(strict=True)
    require(path.parts[1:2] not in (("proc",), ("sys",), ("dev",)), "file_not_text")
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        target = Path("/proc/self/fd") / str(parent) / path.name
        require(stat.S_ISREG(target.lstat().st_mode), "file_not_text")
        fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, "rb") as stream:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode), "file_not_text")
            require(info.st_size <= LIMIT, "file_too_large")
            data = stream.read(LIMIT + 1)
            text = valid_text(data)

            def current():
                require(
                    Path(name).resolve(strict=True) == path
                    and stamp(Path(name).stat())
                    == stamp(info)
                    == stamp(target.lstat()),
                    "target_changed",
                )

            current()
            yield target, Path("/proc/self/fd") / str(fd), path, info, text, current
    finally:
        os.close(parent)


def version(name, path, info, data, instance):
    return revision(
        name,
        str(path),
        info.st_mode,
        info.st_uid,
        info.st_gid,
        digest(data),
        nginx_config.identity(instance),
    )


def view(name, nginx):
    instance = nginx_config.selected(name) if nginx else None
    with opened(name) as (_, _, path, info, text, _):
        return dict(
            path=name,
            resolved=str(path),
            content=text,
            revision=version(name, path, info, text.encode(), instance),
        )


def save(change, content, state):
    name = change["key"]
    instance = nginx_config.selected(name) if change["verb"] == "save_nginx" else None
    require(isinstance(content, str), "invalid_argument")
    data = content.encode()
    valid_text(data)
    require(digest(data) == change["digest"], "invalid_argument")
    with opened(name) as (target, metadata, path, info, text, current):
        previous = text.encode()
        require(
            change["revision"] == version(name, path, info, previous, instance),
            "target_changed",
        )
        require(info.st_nlink == 1, "file_hardlink")
        if previous == data:
            return result("文件内容没有变化")
        # 原子替换还需要目录写权限，同时核实账号对原文件本身有写权限。
        fd = os.open(target, os.O_WRONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            require(stamp(os.fstat(fd)) == stamp(info), "target_changed")
        finally:
            os.close(fd)
        backup = state / ("file-" + digest(os.fsencode(path)) + ".backup")
        atomic(backup, previous)
        installed = atomic(target, data, metadata, before_replace=current)
        if not instance:
            return result("文件已保存")
        info_after = target.lstat()
        require(os.path.samestat(info_after, installed), "target_changed")
        written = stamp(info_after)

        def unchanged():
            require(
                stamp(target.lstat()) == written
                and stamp(Path(name).stat()) == written,
                "target_changed",
            )
            fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(fd, "rb") as candidate:
                require(stamp(os.fstat(fd)) == written, "target_changed")
                require(candidate.read(LIMIT + 1) == data, "target_changed")

        try:
            reason = nginx_config.reload_checked(instance, unchanged)
        except Exception:
            reason = "未能向原 Nginx 实例发送重载信号，请检查进程与 SSH 权限"
        if reason is None:
            return result("已保存并通过 nginx -t，已向主进程发送重载信号")
        try:
            atomic(target, previous, metadata, before_replace=unchanged)
        except Exception:
            return result(f"{reason}。未能恢复原文件，请检查目标并核对备份 {backup}", 1)
        return result(reason + "。已恢复原文件", 1)
