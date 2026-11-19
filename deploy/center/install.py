"""在 Ubuntu 中初始化 Asterion 中心并验收 HTTPS 和自监控。"""

import fcntl
import getpass
import grp
import json
import os
import pwd
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

# 入口使用系统 Python 的隔离模式，只加载与安装器一同交付的模块。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import host
from host import AGENT, CONFIG, RUNTIME, SERVER, UNITS, put, run
from inputs import ask, password, release_info, settings, unpack

STATE = CONFIG / "install.json"
DONE = CONFIG / "install.complete"
BASE = Path("/opt/asterion")
AGENT_BASE = Path("/opt/asterion-agent")
AGENT_CONFIG = Path("/etc/asterion-agent")
FIRST_ADMIN = Path("/run/asterion-first-admin")
ENROLL = Path("/run/asterion-self-enrollment")
IDENTITY = Path("/var/lib/asterion-agent/identity.json")
ADMIN_CONF = RUNTIME / (SERVER + ".d/10-bootstrap.conf")
ENROLL_CONF = RUNTIME / (AGENT + ".d/10-self-enroll.conf")


def fresh(paths):
    for path in paths:
        if os.path.lexists(path):
            raise ValueError(
                f"已有 {path}，首次安装无法接管。已有中心请按部署文档升级。"
            )


def check_fresh():
    fresh(
        [
            CONFIG,
            BASE,
            AGENT_BASE,
            AGENT_CONFIG,
            FIRST_ADMIN,
            ENROLL,
            Path("/run/asterion-setup"),
            Path("/run/asterion-bootstrap"),
            Path("/run/asterion-agent-bootstrap"),
        ]
    )
    for name in ("asterion", "asterion-agent"):
        fresh([Path("/var/lib") / name, Path("/var/lib/private") / name])
    for unit in (SERVER, AGENT):
        fresh(
            [
                base / (unit + suffix)
                for base in (UNITS, RUNTIME)
                for suffix in ("", ".d")
            ]
        )
        if (
            run(
                "systemctl",
                "show",
                "-p",
                "LoadState",
                "--value",
                unit,
                stdout=subprocess.PIPE,
            ).stdout.strip()
            != "not-found"
        ):
            raise ValueError(f"系统已加载 {unit}")


def initialize(revision, digest):
    c = settings()
    fresh(
        [
            host.site_path(c),
            Path("/home") / c["ops"],
            Path("/etc/sudoers.d") / ("asterion-" + c["ops"]),
            host.LE / "renewal-hooks/deploy" / ("asterion-" + c["domain"]),
        ]
    )
    host.check_site(c)
    for lookup in (pwd.getpwnam, grp.getgrnam):
        try:
            lookup(c["ops"])
        except KeyError:
            continue
        raise ValueError("已有同名运维账号或组，请使用新名称")
    if run(
        "ss", "-H", "-lnt", "sport = :" + c["port"], stdout=subprocess.PIPE
    ).stdout.strip():
        raise ValueError("中心内部端口已被占用")
    run("getent", "ahostsv4", c["domain"], stdout=subprocess.DEVNULL)
    print(f"将部署 https://{c['domain']}，管理员为 {c['ops']}-panel。")
    print("请将内部 CA 口令保存到密码管理器，续签回环证书需要它。")
    while True:
        phrase = ask(
            "设置内部 CA 私钥口令（至少 12 位）",
            lambda s: (
                len(s) >= 12 and s.strip() and not any(ch in s for ch in "\r\n\0")
            ),
            secret=True,
        )
        if phrase == getpass.getpass("确认内部 CA 口令："):
            break
        print("两次输入不同。")
    c.update(revision=revision, sha256=digest)
    with tempfile.TemporaryDirectory(dir="/etc", prefix=".asterion-init-") as tmp:
        config = Path(tmp) / "asterion"
        config.mkdir(mode=0o750)
        host.internal_tls(config, phrase)
        put(config / "server.env", host.server_config(c))
        put(config / STATE.name, json.dumps(c, ensure_ascii=False) + "\n")
        config.rename(CONFIG)
    return c


def link(path, target):
    if path.is_symlink() and path.readlink() == target:
        return
    path.symlink_to(target)


def install_release(c, stage):
    releases = BASE / "releases"
    releases.mkdir(parents=True, exist_ok=True)
    release = releases / c["revision"]
    if release.exists():
        if (release / ".sha256").read_text().strip() != c["sha256"]:
            raise ValueError("已有发布目录与本次归档不同")
    else:
        with tempfile.TemporaryDirectory(dir=releases) as tmp:
            dest = Path(tmp) / "release"
            shutil.copytree(stage, dest)
            put(dest / ".sha256", c["sha256"] + "\n")
            dest.rename(release)
    link(BASE / "current", release)
    AGENT_BASE.mkdir(exist_ok=True)
    link(AGENT_BASE / "current", BASE / "current")
    for unit in (SERVER, AGENT):
        put(UNITS / unit, (release / "deploy" / unit).read_text(), 0o644)
    AGENT_CONFIG.mkdir(mode=0o750, exist_ok=True)
    put(
        AGENT_CONFIG / "agent.env",
        f"ASTERION_SERVER_URL=https://127.0.0.1:{c['port']}\n"
        "ASTERION_AGENT_STATE_FILE=/var/lib/asterion-agent/identity.json\n",
    )
    put(
        UNITS / (AGENT + ".d") / "20-local.conf",
        host.template("agent-local.conf"),
        0o644,
    )


def health(c, public=False):
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
        "8",
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


def enable_services(c, secret):
    # 这些运行时路径在首次接管前均已检查，失败后保留身份和持久配置。
    sampled = False
    try:
        FIRST_ADMIN.unlink(missing_ok=True)
        put(FIRST_ADMIN, secret + "\n")
        put(ADMIN_CONF, host.template("admin.conf", ops=c["ops"]), 0o644)
        run("systemctl", "daemon-reload")
        run("systemd-analyze", "verify", SERVER, AGENT)
        run("systemctl", "enable", "--now", SERVER)
        health(c)
        if not IDENTITY.is_file() or not json.loads(IDENTITY.read_text()).get(
            "node_id"
        ):
            run("systemctl", "stop", AGENT)
            monitor(
                "register",
                "--port",
                c["port"],
                "--username",
                c["ops"] + "-panel",
                "--receipt",
                CONFIG / "self-node",
            )
            if (ENROLL / "token").is_file():
                put(ENROLL_CONF, host.template("enroll.conf"), 0o644)
            run("systemctl", "daemon-reload")
        run("systemctl", "enable", AGENT)
        run("systemctl", "restart", AGENT)
        monitor("wait")
        sampled = True
    finally:
        ADMIN_CONF.unlink(missing_ok=True)
        ENROLL_CONF.unlink(missing_ok=True)
        FIRST_ADMIN.unlink(missing_ok=True)
        (ENROLL / "token").unlink(missing_ok=True)
        run("systemctl", "daemon-reload")
        # 重启释放 systemd 的凭据和 RuntimeDirectory 中的一次性副本。
        if sampled:
            run("systemctl", "restart", SERVER, AGENT)
        else:
            run("systemctl", "stop", AGENT, SERVER)
    health(c)
    monitor("wait")


def provision(archive):
    revision, digest = release_info(archive)
    c = None
    if STATE.is_file():
        c = json.loads(STATE.read_text())
        if DONE.is_file():
            health(c, public=True)
            print(
                f"中心已初始化。面板 https://{c['domain']}/login，升级请按部署文档操作。"
            )
            return
        if (revision, digest) != (c["revision"], c["sha256"]):
            raise ValueError("请使用与中断安装相同的发布包继续")
    else:
        check_fresh()
    with tempfile.TemporaryDirectory(prefix="asterion-install-") as tmp:
        work = Path(tmp)
        stage = work / "release"
        print("正在校验并解包发布文件。", flush=True)
        unpack(archive, digest, stage)
        run("apt-get", "update")
        run(
            "apt-get",
            "install",
            "-y",
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
        )
        run("systemctl", "enable", "--now", "nginx")
        if c is None:
            c = initialize(revision, digest)
        else:
            print("继续上次安装。若已建立面板账号，请输入原面板口令。")
        secret = password()
        if not (CONFIG / "install.tls-ready").exists():
            host.public_tls(c)
            put(CONFIG / "install.tls-ready", "\n")
        host.operator(c, work)
        install_release(c, stage)
        enable_services(c, secret)
        host.enable_site(c)
        health(c, public=True)
        for unit in (SERVER, AGENT, "nginx", "certbot.timer"):
            run("systemctl", "is-active", unit)
        put(DONE, "\n")
        print(f"部署完成。面板 https://{c['domain']}/login，用户名 {c['ops']}-panel。")


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
        subprocess.CalledProcessError,
        EOFError,
        KeyboardInterrupt,
    ) as exc:
        sys.exit(f"中心安装未完成：{exc}。请核对上方出错步骤后用同一条命令重试。")
