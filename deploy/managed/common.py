import hashlib
import json
import os
import re
import stat
import subprocess
import tempfile
from pathlib import Path


class Refused(Exception):
    def __init__(self, code="operation_denied"):
        self.code = code


def require(condition, code="operation_denied"):
    if not condition:
        raise Refused(code)


def key_ok(text):
    return isinstance(text, str) and re.fullmatch(
        r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,63}", text
    )


def digest(data):
    return hashlib.sha256(data).hexdigest()


def revision(*items):
    return digest(json.dumps(items, sort_keys=True, ensure_ascii=True).encode())


def fixed_path(value, *, directory=False):
    path = Path(value)
    require(path.is_absolute() and path.resolve(strict=True) == path)
    for parent in path.parents:
        info = parent.stat()
        require(info.st_uid in (0, os.geteuid()) and not info.st_mode & 0o022)
    info = path.lstat()
    require(info.st_uid in (0, os.geteuid()) and not info.st_mode & 0o022)
    require(
        stat.S_ISDIR(info.st_mode)
        if directory
        else stat.S_ISREG(info.st_mode) and info.st_nlink == 1
    )
    return path


def atomic(path, data, original=None, *, before_replace=None):
    fd, temporary = tempfile.mkstemp(dir=path.parent, prefix=".asterion-")
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            if original:
                previous = original.stat()
                os.fchown(stream.fileno(), previous.st_uid, previous.st_gid)
                os.fchmod(stream.fileno(), stat.S_IMODE(previous.st_mode))
                for attribute in os.listxattr(original):
                    os.setxattr(temporary, attribute, os.getxattr(original, attribute))
            os.fsync(stream.fileno())
            written = os.fstat(stream.fileno())
        if before_replace:
            before_replace()
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return written
    finally:
        Path(temporary).unlink(missing_ok=True)


def command(args, timeout=15, *, capture=False):
    with tempfile.TemporaryFile() as output:
        process = subprocess.Popen(
            args,
            stdin=subprocess.DEVNULL,
            stdout=output if capture else subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            **({"group": 0, "extra_groups": []} if os.geteuid() == 0 else {}),
            env={
                "PATH": "/usr/sbin:/usr/bin:/sbin:/bin",
                "LC_ALL": "C",
                "DEBIAN_FRONTEND": "noninteractive",
            },
        )
        try:
            status = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            raise Refused("timeout") from None
        output.seek(0)
        data = output.read(65537)
        require(len(data) <= 65536, "execution_failed")
        return status, data.decode("utf-8").strip()


def result(message, status=0):
    return {
        "exit_code": status,
        "output": message,
        "truncated": False,
        "duration_ms": 0,
    }


def redact(text):
    secret = r"password|passwd|passphrase|authorization:|bearer |token|secret|api[_-]?key|private[_ -]key|client-key|key=|--key|credential|://[^/\s]+@"
    return "\n".join(
        "[已隐藏可能包含凭据的内容]" if re.search(secret, line, re.I) else line
        for line in text.split("\n")
    )
