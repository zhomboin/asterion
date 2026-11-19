import base64
import fcntl
import json
import os
import pty
import re
import socketserver
import struct
import subprocess
import termios
import threading
import time

from common import Refused, key_ok, require
from containers import console_target


class Shell:
    def __init__(self, container, session, cols, rows):
        self.session = session
        self.buffer = bytearray()
        self.offset = 0
        self.opened = self.input_at = time.monotonic()
        self.master, slave = pty.openpty()
        try:
            self.resize(cols, rows)
            self.process = subprocess.Popen(
                ["/usr/bin/docker", "--config", "/var/empty", "--host", "unix:///var/run/docker.sock",
                 "exec", "--interactive", "--tty", "--user", "65534:65534", "--", container,
                 "/usr/bin/timeout", "-k", "2", "900", "/bin/sh", "-i"],
                stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
                env={"PATH": "/usr/bin:/bin", "TERM": "xterm"})
            os.set_blocking(self.master, False)
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)

    def resize(self, cols, rows):
        require(type(cols) is int and type(rows) is int and 20 <= cols <= 400 and 5 <= rows <= 200)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def poll(self, offset):
        require(type(offset) is int and 0 <= offset <= self.offset + len(self.buffer))
        for _ in range(8):
            try:
                data = os.read(self.master, 8192)
            except (BlockingIOError, OSError):
                break
            if not data:
                break
            self.buffer.extend(data)
        excess = max(len(self.buffer) - 65536, 0)
        del self.buffer[:excess]
        self.offset += excess
        start = max(self.offset, offset)
        chunk = self.buffer[start - self.offset:start - self.offset + 8192]
        end = start + len(chunk)
        return {"data": base64.b64encode(chunk).decode(), "offset": end, "truncated": offset < start,
                "closed": self.process.poll() is not None and end == self.offset + len(self.buffer)}

    def close(self):
        try:
            os.write(self.master, b"\x03\x04")
        except OSError:
            pass
        os.close(self.master)
        try:
            self.process.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


class Console(socketserver.StreamRequestHandler):
    def handle(self):
        shell = None
        self.connection.settimeout(125)
        try:
            while header := self.rfile.read(4):
                size, = struct.unpack("!I", header)
                require(0 < size <= 16384)
                request = json.loads(self.rfile.read(size))
                try:
                    now = time.monotonic()
                    if shell and (now - shell.input_at >= 120 or now - shell.opened >= 900):
                        shell.close()
                        shell = None
                    kind, args = request["kind"], request["args"]
                    require(0 < request["expires_at"] - time.time() <= 120)
                    value = None
                    if kind == "terminal_container_capability":
                        value = {"enabled": any(spec.get("terminal") is True for spec in self.server.operator.catalog.values())}
                    elif kind == "terminal_container_open":
                        require(shell is None, "busy")
                        target = args["container"]
                        key = target["key"]
                        require(key_ok(key) and key in self.server.operator.catalog)
                        spec = self.server.operator.catalog[key]
                        require(re.fullmatch(r"[0-9a-f]{32}", args["id"]))
                        require(self.server.operator.lock.acquire(blocking=False), "busy")
                        try:
                            identity = console_target(key, spec, target, self.server.operator.state)
                            shell = Shell(identity, args["id"], args["cols"], args["rows"])
                        finally:
                            self.server.operator.lock.release()
                        value = {"opened": True}
                    else:
                        require(shell and shell.session == args["id"], "terminal_closed")
                        if kind == "terminal_poll":
                            value = shell.poll(args["offset"])
                            if value["closed"]:
                                shell.close()
                                shell = None
                        elif kind == "terminal_input":
                            data = base64.b64decode(args["data"], validate=True)
                            require(0 < len(data) <= 4096)
                            require(os.write(shell.master, data) == len(data))
                            shell.input_at = now
                        elif kind == "terminal_resize":
                            shell.resize(args["cols"], args["rows"])
                        elif kind == "terminal_close":
                            shell.close()
                            shell = None
                        else:
                            raise Refused()
                    reply = {"outcome": "success", "value": value}
                except (Refused, ValueError, TypeError, KeyError, OSError) as error:
                    reply = {"outcome": "failure", "code": getattr(error, "code", "invalid_request"),
                             "message": "容器终端不可用，或目标身份已改变"}
                packet = json.dumps(reply).encode()
                self.wfile.write(struct.pack("!I", len(packet)) + packet)
        except (ValueError, OSError, Refused, struct.error):
            pass
        finally:
            if shell:
                shell.close()


def listen(operator, runtime):
    if not any(spec.get("terminal") is True for spec in operator.catalog.values()):
        return
    require(os.geteuid() != 0)
    path = runtime / "console.sock"
    server = socketserver.UnixStreamServer(str(path), Console)
    server.operator = operator
    os.chmod(path, 0o660)
    threading.Thread(target=server.serve_forever, daemon=True).start()
