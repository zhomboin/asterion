import json
import os
import socketserver
import sys
import threading
import time
from pathlib import Path

import containers
import host
from common import Refused, fixed_path, key_ok, require


class Operator:
    def __init__(self, group, catalog, state):
        require(group in ("packages", "configs", "containers"))
        require(isinstance(catalog, dict) and len(catalog) <= 48 and all(key_ok(key) for key in catalog))
        self.group = group
        self.catalog = catalog
        self.state = state
        self.lock = threading.Lock()

    def handle(self, request):
        if request.get("query") is True:
            key = request.get("key")
            if key is None:
                return {"keys": sorted(self.catalog)}
            require(key_ok(key) and key in self.catalog)
            spec = self.catalog[key]
            if self.group == "configs":
                return host.config_view(key, spec)
            if self.group == "packages":
                return host.package_view(key, spec)
            return containers.view(key, spec, self.state)
        change = request["change"]
        key = change["key"]
        require(key_ok(key) and key in self.catalog)
        require(0 < request["expires_at"] - time.time() <= 931, "timeout")
        verb = change["verb"]
        spec = self.catalog[key]
        if self.group == "configs" and verb == "save_reload":
            return host.save(key, spec, change, request["content"], self.state)
        if self.group == "packages" and verb == "install":
            return host.install(key, spec, change)
        require(self.group == "containers")
        return containers.apply(key, spec, change, self.state)


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(2)
        try:
            raw = self.rfile.readline(53249)
            require(raw.endswith(b"\n") and len(raw) <= 53248)
            start = time.monotonic()
            if not self.server.operator.lock.acquire(blocking=False):
                raise Refused("agent_busy")
            try:
                value = self.server.operator.handle(json.loads(raw))
                if "exit_code" in value:
                    value["duration_ms"] = round((time.monotonic() - start) * 1000)
            finally:
                self.server.operator.lock.release()
            response = {"result": value}
        except Refused as failure:
            response = {"error": failure.code}
        except (KeyError, ValueError, TypeError, OSError, RecursionError):
            response = {"error": "invalid_argument"}
        try:
            self.wfile.write(json.dumps(response, ensure_ascii=False).encode() + b"\n")
        except OSError:
            pass


def main():
    group = sys.argv[1]
    runtime = fixed_path(os.environ["RUNTIME_DIRECTORY"], directory=True)
    state = fixed_path(Path(os.environ["STATE_DIRECTORY"]).resolve(strict=True), directory=True)
    require(state.stat().st_mode & 0o077 == 0)
    policy = Path(os.environ["CREDENTIALS_DIRECTORY"]) / "policy.json"
    require(policy.stat().st_size <= 65536)
    operator = Operator(group, json.loads(policy.read_bytes()), state)
    address = runtime / "requests.sock"
    with socketserver.UnixStreamServer(str(address), Handler) as server:
        server.operator = operator
        os.chmod(address, 0o660)
        if group == "containers":
            import console
            console.listen(operator, runtime)
        server.serve_forever()


if __name__ == "__main__":
    main()
