"""读取远端已有 Docker Engine 的本地接口。"""

import datetime
import http.client
import json
import re
import socket
import struct

from common import Refused, require


class Engine(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(5)
        self.sock.connect("/var/run/docker.sock")


def fetch(path, raw=False):
    connection = Engine("localhost")
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        require(
            response.status == 200,
            "not_found" if response.status == 404 else "unavailable",
        )
        data = response.read(2**20 + 1)
        require(len(data) <= 2**20, "too_large")
        return data if raw else json.loads(data)
    finally:
        connection.close()


def listing(args):
    try:
        info = fetch("/info")
        candidates = fetch("/containers/json?all=1")
    except (OSError, Refused) as error:
        status = (
            "permission_denied"
            if isinstance(error, PermissionError)
            else "not_installed"
            if isinstance(error, FileNotFoundError)
            else "unreachable"
        )
        return dict(
            status=status,
            containers=[],
            engine_id=None,
            engine_version=None,
            endpoint_fingerprint=None,
        )
    rows = []
    for item in candidates:
        cid = item["Id"]
        if args.get("ids") is not None and cid not in args["ids"]:
            continue
        name = (item.get("Names") or [cid])[0].lstrip("/")
        if (
            args.get("q", "").lower()
            not in " ".join((name, cid, item["Image"])).lower()
        ):
            continue
        detail = fetch("/containers/" + cid + "/json")
        state = detail["State"]
        labels = item.get("Labels") or {}
        try:
            started = int(
                datetime.datetime.fromisoformat(
                    state["StartedAt"][:19] + "+00:00"
                ).timestamp()
            )
        except (ValueError, OverflowError, KeyError):
            started = None
        rows.append(
            dict(
                id=cid,
                name=name,
                image=item["Image"],
                state=item["State"],
                status=item["Status"],
                health=state.get("Health", {}).get("Status"),
                started_at=started,
                restart_count=detail["RestartCount"],
                compose_project=labels.get("com.docker.compose.project"),
                compose_service=labels.get("com.docker.compose.service"),
                mounts=[
                    dict(
                        source=m["Source"],
                        destination=m["Destination"],
                        mode="rw" if m["RW"] else "ro",
                    )
                    for m in item.get("Mounts", [])[:32]
                ],
                ports=[
                    dict(
                        ip=p.get("IP"),
                        private_port=p["PrivatePort"],
                        public_port=p.get("PublicPort"),
                        protocol=p["Type"],
                    )
                    for p in item.get("Ports", [])[:32]
                ],
            )
        )
        if len(rows) >= 64:
            break
    return dict(
        status="available",
        engine_id=info["ID"],
        engine_version=info.get("ServerVersion"),
        engine_kind="desktop"
        if "Docker Desktop" in info.get("OperatingSystem", "")
        else "native",
        containers=rows,
        truncated=len(rows) >= 64,
        endpoint_fingerprint=None,
    )


def query(kind, args):
    if kind == "docker":
        return listing(args)
    cid = args["id"]
    require(re.fullmatch("[a-f0-9]{64}", cid))
    path = "/containers/" + cid
    if kind == "docker_logs":
        data = fetch(
            path + "/logs?stdout=1&stderr=1&tail=" + str(int(args["lines"])), raw=True
        )
        if not fetch(path + "/json")["Config"].get("Tty"):
            offset, chunks = 0, []
            while offset + 8 <= len(data):
                (length,) = struct.unpack_from("!I", data, offset + 4)
                chunks.append(data[offset + 8 : offset + 8 + length])
                offset += 8 + length
            data = b"".join(chunks)
        from actions import redact

        return dict(
            text=redact(data[-32768:].decode(errors="replace")),
            truncated=len(data) > 32768,
        )
    value = fetch(path + "/stats?stream=false")
    now, old = value["cpu_stats"], value.get("precpu_stats", {})

    def usage(c):
        return c.get("cpu_usage", {}).get("total_usage", 0)

    total = now.get("system_cpu_usage", 0) - old.get("system_cpu_usage", 0)
    mem = value.get("memory_stats", {})
    cached = mem.get("stats", {})
    nets = list(value.get("networks", {}).values())
    return dict(
        cpu_percent=(usage(now) - usage(old)) * 100 * now.get("online_cpus", 1) / total
        if old and total > 0
        else None,
        memory_bytes=max(
            0,
            mem["usage"]
            - cached.get("total_inactive_file", cached.get("inactive_file", 0)),
        )
        if "usage" in mem
        else None,
        memory_limit_bytes=mem.get("limit"),
        rx_bytes=sum(i["rx_bytes"] for i in nets),
        tx_bytes=sum(i["tx_bytes"] for i in nets),
    )
