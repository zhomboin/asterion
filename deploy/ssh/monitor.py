"""通过 SSH 按需读取 Linux 内核和 systemd 数据。"""

import os
import platform
import pwd
import re
import time
from pathlib import Path

from common import command, redact, require


def text(path):
    with open(path, errors="replace") as stream:
        return stream.read(2**20)


def counters():
    ticks = list(map(int, text("/proc/stat").splitlines()[0].split()[1:9]))
    interfaces = {}
    for row in text("/proc/net/dev").splitlines()[2:]:
        name, data = row.split(":", 1)
        fields = data.split()
        interfaces[name.strip()] = int(fields[0]), int(fields[8])
    return sum(ticks), sum(ticks[3:5]), interfaces


def filesystems():
    result = []
    virtual = {
        "proc",
        "sysfs",
        "tmpfs",
        "devtmpfs",
        "devpts",
        "cgroup",
        "cgroup2",
        "squashfs",
        "ramfs",
        "autofs",
        "mqueue",
        "debugfs",
        "tracefs",
    }
    for line in text("/proc/self/mounts").splitlines():
        _, mount, kind, *_ = line.split()
        if mount != "/" and (
            kind in virtual or mount.startswith(("/sys/", "/proc/", "/dev/"))
        ):
            continue
        path = re.sub(r"\\([0-7]{3})", lambda m: chr(int(m[1], 8)), mount)
        if not path.isprintable():
            continue
        try:
            fs = os.statvfs(path)
            if fs.f_blocks:
                result.append(
                    dict(
                        mountpoint=path,
                        fs_type=kind,
                        total_bytes=fs.f_blocks * fs.f_frsize,
                        available_bytes=max(0, fs.f_bavail) * fs.f_frsize,
                    )
                )
        except OSError:
            continue
        if len(result) >= 64:
            break
    return result


def sample():
    before, idle, net = counters()
    start = time.monotonic()
    time.sleep(0.25)
    after, idle_now, network = counters()
    seconds = time.monotonic() - start
    mem = {
        k: int(v.split()[0]) * 1024
        for k, v in (line.split(":", 1) for line in text("/proc/meminfo").splitlines())
    }

    def memory(total, available):
        free = min(mem[available], mem[total])
        return dict(
            total_bytes=mem[total], available_bytes=free, used_bytes=mem[total] - free
        )

    rows = []
    for name, (rx, tx) in list(network.items())[:128]:
        old = net.get(name)
        rows.append(
            dict(
                name=name,
                rx_bytes=rx,
                tx_bytes=tx,
                rx_bytes_per_second=(rx - old[0]) / seconds
                if old and rx >= old[0]
                else None,
                tx_bytes_per_second=(tx - old[1]) / seconds
                if old and tx >= old[1]
                else None,
            )
        )
    cpu = (
        max(0, min(100, 100 * (1 - (idle_now - idle) / (after - before))))
        if after > before
        else None
    )
    return dict(
        os=platform.system(),
        arch=platform.machine(),
        wsl_distro=os.getenv("WSL_DISTRO_NAME"),
        sample=dict(
            captured_at=int(time.time()),
            logical_cores=os.cpu_count() or 1,
            cpu_percent=cpu,
            load=os.getloadavg(),
            memory=memory("MemTotal", "MemAvailable"),
            swap=memory("SwapTotal", "SwapFree"),
            filesystems=filesystems(),
            interfaces=rows,
        ),
    )


def process(pid, detail=False):
    path = Path("/proc", str(pid))
    line = text(path / "stat")
    fields = line[line.rindex(")") + 2 :].split()
    name = line[line.index("(") + 1 : line.rindex(")")]
    hz = os.sysconf("SC_CLK_TCK")
    start = int(fields[19])
    uptime = float(text("/proc/uptime").split()[0])
    age = uptime - start / hz
    row = dict(
        pid=pid,
        name=name,
        start_ticks=start,
        state=fields[0],
        cpu_percent=(int(fields[11]) + int(fields[12])) / hz * 100 / age
        if age > 0
        else None,
        memory_bytes=max(0, int(fields[21])) * os.sysconf("SC_PAGE_SIZE"),
        started_at=int(time.time() - age),
    )
    if detail:
        try:
            executable = os.readlink(path / "exe")
        except OSError:
            executable = None
        cgroup = text(path / "cgroup")
        unit = re.search(r"/([^/\n]+\.service)(?:/|$)", cgroup, re.M)
        container = re.search(r"(?:docker[-/])([0-9a-f]{64})", cgroup)
        runtime = (
            dict(kind="docker", container_id=container[1])
            if container
            else (dict(kind="systemd", unit=unit[1]) if unit else None)
        )
        uid = path.stat().st_uid
        try:
            user = pwd.getpwuid(uid).pw_name
        except KeyError:
            user = str(uid)
        try:
            with (path / "cmdline").open("rb") as stream:
                args = (
                    stream.read(4096).decode(errors="replace").rstrip("\0").split("\0")
                )
            command_line = []
            hide_next = False
            for argument in args:
                safe = redact(argument)
                command_line.append("[已隐藏]" if hide_next else safe)
                hide_next = (
                    argument.startswith("-")
                    and "=" not in argument
                    and safe != argument
                )
        except OSError:
            command_line = None
        row.update(
            parent_pid=int(fields[1]),
            command_line=command_line,
            executable=executable,
            user=user,
            runtime=runtime,
        )
        require(start == process(pid)["start_ticks"], "not_found")
    return row


def processes(args):
    rows = []
    search = args.get("q", "").lower()
    names = args.get("names")
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            row = process(int(pid))
        except (OSError, ValueError, IndexError):
            continue
        if (names is None or row["name"] in names) and (
            search in row["name"].lower() or search in pid
        ):
            rows.append(row)
    sort = args.get("sort", "cpu")
    key = {"cpu": "cpu_percent", "memory": "memory_bytes"}.get(sort, sort)
    rows.sort(
        key=lambda r: r[key] if r[key] is not None else 0,
        reverse=sort in ("cpu", "memory"),
    )
    limit = args.get("limit", 100)
    if names:
        return [
            r for name in names for r in [r for r in rows if r["name"] == name][:limit]
        ]
    start = args.get("offset", 0)
    return rows[start : start + limit]


def unit_row(name, active, sub, description, props=None):
    props = props or {}
    program = re.search(r"path=([^ ;]+)", props.get("ExecStart", ""))
    return dict(
        name=name,
        active=active,
        sub=sub,
        description=description[:512],
        source="systemd",
        observed_at=int(time.time()),
        verified=bool(props),
        exec_start=program[1] if program else None,
        working_directory=props.get("WorkingDirectory"),
        fragment_path=props.get("FragmentPath"),
    )


def systemd(args):
    base = [
        "/usr/bin/systemctl",
        "--system",
        "--no-pager",
        "--no-ask-password",
        "--full",
    ]
    if "unit" in args:
        name = args["unit"]
        status, output = command(
            base
            + [
                "show",
                "-p",
                "Id,LoadState,ActiveState,SubState,Description,ExecStart,WorkingDirectory,FragmentPath",
                "--",
                name,
            ],
            capture=True,
        )
        props = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
        require(
            status == 0
            and props.get("LoadState") != "not-found"
            and props.get("Id") == name,
            "not_found",
        )
        return unit_row(
            name,
            props["ActiveState"],
            props["SubState"],
            props.get("Description", ""),
            props,
        )
    if not Path("/run/systemd/system").is_dir():
        return dict(status="unavailable", units=[])
    status, output = command(
        base + ["list-units", "--all", "--type=service", "--plain", "--no-legend"],
        capture=True,
    )
    rows = []
    for line in output.splitlines():
        items = line.strip().lstrip("● ").split(None, 4)
        if len(items) >= 4 and args.get("q", "").lower() in line.lower():
            rows.append(
                unit_row(
                    items[0], items[2], items[3], items[4] if len(items) > 4 else ""
                )
            )
    if status:
        return dict(status="unavailable", units=[])
    seen = {row["name"] for row in rows}
    code, files = command(
        base + ["list-unit-files", "--type=service", "--no-legend"], capture=True
    )
    if code == 0:
        for line in files.splitlines():
            fields = line.split()
            if not fields:
                continue
            name = fields[0]
            if (
                name.endswith(".service")
                and not name.endswith("@.service")
                and name not in seen
                and args.get("q", "").lower() in name.lower()
            ):
                rows.append(unit_row(name, "unknown", "unknown", ""))
                seen.add(name)
    return dict(status="available", units=rows[:200], truncated=len(rows) > 200)
