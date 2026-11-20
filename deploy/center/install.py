"""核对并补齐 Ubuntu 中心的发布文件、配置及服务。"""

import fcntl
import json
import os
import platform
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import existing
import host
import pki
import services
from host import AGENT, CONFIG, SERVER, UNITS, put, run
from inputs import release_info, settings, unpack

STATE = CONFIG / "install.json"
DONE = CONFIG / "install.complete"
BASE = Path("/opt/asterion")
AGENT_BASE = Path("/opt/asterion-agent")
AGENT_CONFIG = Path("/etc/asterion-agent")
SETUP = Path("/run/asterion-setup")


def runtime_tools():
    packages = [
        "nginx",
        "openssl",
        "ca-certificates",
        "curl",
        "python3-paramiko",
        "sqlite3",
        "sudo",
        "openssh-client",
        "certbot",
        "python3-certbot-dns-cloudflare",
    ]
    missing = []
    for package in packages:
        result = subprocess.run(
            ["dpkg-query", "-W", "-f=${db:Status-Status}", package],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.stdout != "installed":
            missing.append(package)
    if missing:
        run("apt-get", "update")
        run("apt-get", "install", "-y", *missing)


def check_binaries(stage):
    # 使用系统加载器核对动态依赖及 glibc 版本，无需运行服务或安装编译器。
    loaders = {
        "x86_64": (62, "/lib64/ld-linux-x86-64.so.2"),
        "aarch64": (183, "/lib/ld-linux-aarch64.so.1"),
    }
    for name in ("asterion-server", "asterion-agent"):
        binary = stage / "bin" / name
        with binary.open("rb") as stream:
            header = stream.read(20)
        arch = loaders.get(platform.machine())
        if (
            not arch
            or len(header) != 20
            or header[:6] != b"\x7fELF\x02\x01"
            or int.from_bytes(header[18:], "little") != arch[0]
        ):
            raise ValueError(f"{name} 与目标 CPU 架构不兼容，请选用适配本机的发布包")
        result = subprocess.run(
            [arch[1], "--list", binary], text=True, capture_output=True, check=False
        )
        if result.returncode:
            raise ValueError(
                f"{name} 无法加载，请选用目标系统构建的发布包：{result.stderr.strip()}"
            )


def check_layout(c, stage):
    for path in (BASE, BASE / "releases", AGENT_BASE, CONFIG, AGENT_CONFIG):
        existing.trusted(path, directory=True)
    release = BASE / "releases" / c["revision"]
    existing.trusted(release, directory=True)
    existing.check_link(BASE / "current", release)
    existing.check_link(AGENT_BASE / "current", BASE / "current")
    services.check_units(c, stage)
    for path, expected in (
        (CONFIG / "server.env", host.server_config(c)),
        (AGENT_CONFIG / "agent.env", agent_config(c)),
    ):
        values = existing.env_values(path)
        for line in expected.splitlines():
            key, value = line.split("=", 1)
            if values.get(key, value) != value:
                raise ValueError(f"{path} 中 {key} 与目标配置不一致")


def agent_config(c):
    return f"ASTERION_SERVER_URL=https://127.0.0.1:{c['port']}\nASTERION_AGENT_STATE_FILE=/var/lib/asterion-agent/identity.json\n"


def install_release(c, stage):
    releases = BASE / "releases"
    BASE.mkdir(mode=0o755, exist_ok=True)
    releases.mkdir(mode=0o755, exist_ok=True)
    release = releases / c["revision"]
    if release.exists():
        existing.reconcile_release(stage, release, c["sha256"], put)
        print("已核对并复用解包目录。")
    else:
        with tempfile.TemporaryDirectory(dir=releases) as tmp:
            dest = Path(tmp) / "release"
            shutil.copytree(stage, dest)
            put(dest / ".sha256", c["sha256"] + "\n")
            dest.rename(release)
    AGENT_BASE.mkdir(mode=0o755, exist_ok=True)
    for path, target in (
        (BASE / "current", release),
        (AGENT_BASE / "current", BASE / "current"),
    ):
        existing.check_link(path, target)
        if not path.is_symlink():
            path.symlink_to(target)
    for unit in (SERVER, AGENT):
        host.unit_file(UNITS / unit, (release / "deploy" / unit).read_text())
    AGENT_CONFIG.mkdir(mode=0o750, exist_ok=True)
    existing.ensure_env(AGENT_CONFIG / "agent.env", agent_config(c), put)
    host.unit_file(
        UNITS / (AGENT + ".d") / "20-local.conf",
        host.template("agent-local.conf"),
    )


def health(c, public=False, retry=True):
    args = [
        "curl",
        "--noproxy",
        "*",
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        "5",
        "--retry",
        "8" if retry else "0",
        "--retry-delay",
        "1",
        "--retry-all-errors",
    ]
    if public:
        domain = c["domain"]
        args += ["--resolve", f"{domain}:443:127.0.0.1", f"https://{domain}/healthz"]
    else:
        args += [
            "--cacert",
            CONFIG / "deploy-ca.crt",
            f"https://127.0.0.1:{c['port']}/healthz",
        ]
    if json.loads(run(*args, stdout=subprocess.PIPE).stdout).get("status") != "ok":
        raise ValueError("中心健康检查未通过")


def monitor(*args):
    run("/usr/bin/python3", "-I", "-B", host.HERE.parent / "self_monitor.py", *args)


def provision(archive):
    revision, digest = release_info(archive)
    existing.trusted(CONFIG, directory=True)
    recorded = existing.trusted(STATE, private=True)
    existing.trusted(DONE)
    with tempfile.TemporaryDirectory(prefix="asterion-install-") as tmp:
        work = Path(tmp)
        stage = work / "release"
        print("正在校验并解包发布文件。", flush=True)
        unpack(archive, digest, stage)
        check_binaries(stage)
        if recorded:
            c = json.loads(STATE.read_text())
            if (revision, digest) != (c["revision"], c["sha256"]):
                raise ValueError("发布包与安装记录不同；更新已安装版本请使用升级流程")
        else:
            c = settings(existing.hints(CONFIG, host.SITES, host.SUDOERS))
            c.update(revision=revision, sha256=digest)
        check_layout(c, stage)
        existing.login_account(c["ops"])
        if (
            not services.active(SERVER)
            and run(
                "ss", "-H", "-lnt", "sport = :" + c["port"], stdout=subprocess.PIPE
            ).stdout.strip()
        ):
            raise ValueError(f"回环端口 {c['port']} 被其他服务占用")
        runtime_tools()
        host.check_existing_site(c)
        CONFIG.mkdir(mode=0o750, exist_ok=True)
        install_release(c, stage)
        pki.ensure_tls(CONFIG, SETUP, work)
        existing.ensure_env(CONFIG / "server.env", host.server_config(c), put)
        if not recorded:
            put(STATE, json.dumps(c, ensure_ascii=False) + "\n")
        host.operator(c, work, SETUP)
        tls_marker = CONFIG / "install.tls-ready"
        host.public_tls(c, verified=existing.trusted(tls_marker))
        put(tls_marker, "\n")
        services.ensure_running("nginx")
        services.activate(c, health, monitor)

        def live():
            try:
                health(c, public=True, retry=False)
                return True
            except (ValueError, subprocess.CalledProcessError):
                return False

        host.enable_site(c, live)
        health(c, public=True)
        for unit in (SERVER, AGENT, "nginx", "certbot.timer"):
            run("systemctl", "is-active", unit)
        put(DONE, "\n")
        print(
            f"部署完成。面板 https://{c['domain']}/login，用户名 {existing.admin_name(services.DATABASE)}。"
        )


def main():
    if os.geteuid() != 0 or len(sys.argv) != 2:
        sys.exit("请从 bash deploy/install-center.sh 运行。")
    os.umask(0o022)
    lock_fd = os.open(
        "/run/asterion-center-install.lock",
        os.O_CREAT | os.O_WRONLY | os.O_NOFOLLOW,
        0o600,
    )
    with os.fdopen(lock_fd, "w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            sys.exit("另一个中心安装进程正在运行。")
        provision(Path(sys.argv[1]))


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        tarfile.TarError,
        sqlite3.Error,
        subprocess.CalledProcessError,
        EOFError,
        KeyboardInterrupt,
    ) as exc:
        sys.exit(f"中心安装未完成：{exc}。请核对上方出错步骤后用同一条命令重试。")
