"""在一次性目录中测试解压、首次口令、登录和再次启动。"""

import fcntl
import http.cookiejar
import json
import os
from pathlib import Path
import pty
import re
import secrets
import select
import signal
import socket
import subprocess
import sys
import tempfile
import termios
import time
import urllib.request
from urllib.parse import unquote, urlsplit


def check_guides(work):
    docs = work / "docs"
    assert (docs / "local.md").is_file()
    for guide in docs.glob("*.md"):
        for link in re.findall(r"\]\(([^)\s]+)\)", guide.read_text(encoding="utf-8")):
            target = urlsplit(link)
            if not target.scheme and not target.netloc and target.path:
                assert (guide.parent / unquote(target.path)).is_file(), (
                    guide.name,
                    link,
                )


def expect(fd, text):
    buffer = bytearray()
    end = time.monotonic() + 20
    while time.monotonic() < end:
        if select.select([fd], [], [], 0.3)[0]:
            buffer.extend(os.read(fd, 4096))
            if text.encode() in buffer:
                return
    raise AssertionError("启动终端未显示：" + text)


def check(work, port, password, first):
    master, slave = pty.openpty()

    def terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = {k: v for k, v in os.environ.items() if not k.startswith("ASTERION_")}
    process = subprocess.Popen(
        ["bash", "start.sh"],
        cwd=work,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env={**env, "PATH": "/usr/bin:/bin", "ASTERION_LOCAL_PORT": str(port)},
        preexec_fn=terminal,
    )
    os.close(slave)
    try:
        if first:
            for prompt in ("请设置本机面板口令", "请再次输入口令"):
                expect(master, prompt)
                os.write(master, (password + "\n").encode())
        expect(master, "Ctrl+C 退出")
        base = f"http://127.0.0.1:{port}"
        opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
        )
        with opener.open(base + "/login", timeout=5) as page:
            assert b'<div id="root">' in page.read()
        req = urllib.request.Request(
            base + "/api/v1/auth/login",
            data=json.dumps(dict(username="local", password=password)).encode(),
            headers={"Content-Type": "application/json", "X-Asterion-CSRF": "1"},
        )
        with opener.open(req, timeout=5) as response:
            assert json.load(response)["username"] == "local"
            opener.addheaders = [
                (
                    "X-Asterion-Local-Session",
                    response.headers["X-Asterion-Local-Session"],
                )
            ]
        with opener.open(base + "/api/v1/settings", timeout=5) as response:
            assert json.load(response)["local_ssh"]
        os.write(master, b"\x03")
        assert process.wait(timeout=10) == 0
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        os.close(master)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(dir=".local") as tmp:
        work = Path(tmp).resolve()
        subprocess.run(["tar", "-xzf", sys.argv[1], "-C", work], check=True)
        check_guides(work)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        password = secrets.token_hex(16)
        check(work, port, password, True)
        check(work, port, password, False)
    print("已验证解压运行、首次口令、登录、停止及重启")
