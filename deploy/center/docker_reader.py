"""在中心主机上按需补齐 Docker 只读采集。"""

import grp
import os
import pwd
import shutil
import stat
import time
from pathlib import Path

import existing
import host
import services
from inputs import ask

UNIT = "asterion-docker-reader.service"
ACCOUNT = "asterion-docker"
GROUP = "asterion-readers"
ENGINE = Path("/run/docker.sock")
SOCKET = "/run/asterion-docker-reader/requests.sock"
VARIABLE = "ASTERION_DOCKER_READER_SOCKET"


def detect():
    try:
        info = ENGINE.lstat()
    except FileNotFoundError:
        if any(Path("/run/user").glob("*/docker.sock")):
            return "skip", "检测到用户级 Docker Socket；请按部署指南配置对应端点。"
        if services.property_value("docker.service", "LoadState") == "loaded":
            if not services.active("docker.service"):
                return (
                    "pending",
                    "检测到已安装但未运行的系统 Docker，可先配置采集，待 Engine 启动后读取。",
                )
            return (
                "skip",
                "Docker 正在运行但没有标准 Socket，请按部署指南配置实际端点。",
            )
        if shutil.which("docker") or shutil.which("dockerd"):
            return (
                "skip",
                "检测到 Docker 程序，但未找到本机系统 Engine 的标准 Socket 或服务。",
            )
        return "skip", "未检测到本机 Docker Engine，跳过只读采集。"

    try:
        docker_gid = grp.getgrnam("docker").gr_gid
    except KeyError:
        docker_gid = None
    if (
        not stat.S_ISSOCK(info.st_mode)
        or info.st_uid != 0
        or not docker_gid
        or info.st_gid != docker_gid
        or not info.st_mode & stat.S_IWGRP
        or info.st_mode & stat.S_IWOTH
    ):
        return (
            "skip",
            "标准 Docker Socket 的类型、属主或权限不适用，请按部署指南核对实际端点和访问组。",
        )
    return "ready", "检测到本机系统 Docker Socket。"


def reader_group():
    try:
        docker_gid = grp.getgrnam("docker").gr_gid
    except KeyError:
        raise ValueError("系统 Docker 缺少 docker 组，请核对 Engine 安装") from None
    try:
        group = grp.getgrnam(GROUP)
    except KeyError:
        group = None
    try:
        account = pwd.getpwnam(ACCOUNT)
    except KeyError:
        account = None
    if not docker_gid or (group and group.gr_gid in (0, docker_gid)):
        raise ValueError("Docker 与读取组需要独立的非 root 组")
    if account and (
        not group
        or not 0 < account.pw_uid < 1000
        or account.pw_gid != group.gr_gid
        or account.pw_shell not in ("/usr/sbin/nologin", "/sbin/nologin", "/bin/false")
    ):
        raise ValueError(f"已有同名账号 {ACCOUNT} 与专用读取服务账号不符")
    if not group:
        host.run("groupadd", "--system", GROUP)
        group = grp.getgrnam(GROUP)
    if not account:
        host.run(
            "useradd",
            "--system",
            "--gid",
            GROUP,
            "--no-create-home",
            "--shell",
            "/usr/sbin/nologin",
            ACCOUNT,
        )
    return group.gr_gid


def installed_release(base, agent_base, config):
    current = base / "current"
    if not current.is_symlink():
        raise ValueError("找不到中心的当前发布，请选择完整中心部署")
    release = base / current.readlink()
    if release.parent != base / "releases" or release.name == "..":
        raise ValueError("中心发布链接未直接指向已安装发布目录，请核对现有配置")
    for directory in (
        base,
        base / "releases",
        release,
        release / "bin",
        agent_base,
        config,
    ):
        if not existing.trusted(directory, directory=True):
            raise ValueError(f"中心安装缺少目录 {directory}，请选择完整中心部署")
    agent = agent_base / "current"
    if not agent.is_symlink() or agent_base / agent.readlink() not in (
        current,
        release,
    ):
        raise ValueError("本机 Agent 的发布链接与中心不一致，请核对现有配置")
    if not existing.trusted(host.UNITS / host.AGENT) or not services.active(host.AGENT):
        raise ValueError("中心本机 Agent 尚未运行，请选择完整中心部署")
    if not existing.trusted(config / "agent.env", private=True):
        raise ValueError("中心本机 Agent 缺少 agent.env，请选择完整中心部署")
    services.check_unit(
        host.AGENT, host.HERE.parent / host.AGENT, services.agent_overrides()
    )
    return release


def check_install(base, agent_base, config):
    release = installed_release(base, agent_base, config)
    binary = release / "bin/asterion-docker-reader"
    if not existing.trusted(binary) or not os.access(binary, os.X_OK):
        raise ValueError(
            "当前 Agent 发布缺少可执行的 Docker 读取程序，请按部署指南补齐发布"
        )
    if existing.env_values(config / "agent.env").get(VARIABLE, SOCKET) != SOCKET:
        raise ValueError("agent.env 已指定其他 Docker 读取端点，请核对现有配置")
    services.check_unit(UNIT, host.HERE.parent / UNIT, {})


def process_access(required_groups, variable, value):
    pid = services.property_value(host.AGENT, "MainPID")
    if not pid.isdecimal() or int(pid) == 0:
        return None
    proc = Path("/proc") / pid
    try:
        fields = dict(
            line.split(":", 1) for line in (proc / "status").read_text().splitlines()
        )
        env = (proc / "environ").read_bytes().split(b"\0")
    except (FileNotFoundError, ProcessLookupError):
        return None
    uid = int(fields["Uid"].split()[1])
    gid = int(fields["Gid"].split()[1])
    groups = list(map(int, fields["Groups"].split()))
    if (
        uid
        and set(required_groups) <= set(groups)
        and f"{variable}={value}".encode() in env
    ):
        return {"user": uid, "group": gid, "extra_groups": groups}
    return None


def agent_access(reader_gid):
    return process_access([reader_gid], VARIABLE, SOCKET)


def configure(base, agent_base, config, *, selected=False):
    state, message = detect()
    print(message, flush=True)
    if state == "skip":
        return
    if (
        not selected
        and ask(
            "本机 Docker 只读采集（1 启用或补齐，权限变更时重启本机 Agent；2 跳过并保留当前配置）",
            lambda value: value in ("1", "2"),
            "2",
        )
        != "1"
    ):
        return
    check_install(base, agent_base, config)
    gid = reader_group()
    host.unit_file(host.UNITS / UNIT, (host.HERE.parent / UNIT).read_text())
    host.unit_file(services.DOCKER_CONF, host.template("docker-read.conf"))
    existing.ensure_env(config / "agent.env", f"{VARIABLE}={SOCKET}\n", host.put)
    host.run("systemctl", "daemon-reload")
    host.run("systemd-analyze", "verify", UNIT, host.AGENT)
    services.ensure_running(UNIT)
    access = agent_access(gid)
    if not access:
        services.ensure_running(host.AGENT, restart=True)
        for _ in range(25):
            access = agent_access(gid)
            if access:
                break
            time.sleep(0.2)
    if not access:
        raise ValueError("Agent 尚未取得读取组或 Socket 配置，请核对服务后重试")
    # 以正在运行的 Agent 的实际 UID、GID 和附加组访问读取服务。
    with (host.HERE / "docker_probe.py").open() as probe:
        host.run(
            "/usr/bin/python3",
            "-I",
            "-B",
            "-",
            SOCKET,
            state,
            stdin=probe,
            env={},
            cwd="/",
            **access,
        )
