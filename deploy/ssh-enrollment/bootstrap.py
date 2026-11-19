import base64
import contextlib
import hashlib
import hmac
import io
import ipaddress
import json
import os
import re
import shlex
import socket
import struct
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit


class Failure(Exception):
    pass


def require(test, message):
    if not test:
        raise Failure(message)


def emit(**fields):
    print(json.dumps(fields, ensure_ascii=False), flush=True)


def destination(host, port, ranges, pinned=None):
    networks = [ipaddress.ip_network(item.strip(), strict=True) for item in ranges.split(",")]
    addresses = {entry[4][0] for entry in socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)}
    require(addresses and all(any(ipaddress.ip_address(addr) in net for net in networks)
                              for addr in addresses), "目标地址不在中心允许的部署网段内")
    if pinned is not None:
        require(pinned in addresses, "目标地址已变化，请重新核对指纹")
        return pinned
    return sorted(addresses)[0]


def origin(value):
    require(re.fullmatch(r"https://[A-Za-z0-9.\[\]:-]+/?", value), "中心未配置有效的 HTTPS 回连地址")
    parsed = urlsplit(value)
    require(parsed.hostname and (parsed.port is None or 0 < parsed.port <= 65535),
            "中心回连地址无效")
    return value.rstrip("/")


def arch_of(binary):
    with binary.open("rb") as stream:
        header = stream.read(20)
    require(len(header) == 20 and header[:6] == b"\x7fELF\x02\x01", "发布产物不是受支持的 Linux Agent")
    arch = {62: "x86_64", 183: "aarch64"}.get(struct.unpack_from("<H", header, 18)[0])
    require(arch, "发布产物架构不受支持")
    return arch


def fingerprint(key):
    return "SHA256:" + base64.b64encode(hashlib.sha256(key.asbytes()).digest()).decode().rstrip("=")


def authenticate(ssh, req, paramiko):
    if req["method"] == "password":
        ssh.auth_password(req["user"], req["secret"], fallback=False)
    else:
        key = None
        for cls in (paramiko.Ed25519Key, paramiko.ECDSAKey, paramiko.RSAKey):
            try:
                key = cls.from_private_key(io.StringIO(req["secret"]), password=req.get("passphrase"))
                break
            except (ValueError, paramiko.SSHException):
                continue
        require(key is not None, "无法读取 SSH 私钥，请检查格式或解密口令")
        ssh.auth_publickey(req["user"], key)
    require(ssh.is_authenticated(), "SSH 身份验证未完成")


APP = "/opt/asterion-agent"
CONF = "/etc/asterion-agent"
UNIT = "asterion-agent.service"
SYSTEM = "/etc/systemd/system/" + UNIT
BOOT = "/run/asterion-enrollment"
DROP = "/run/systemd/system/" + UNIT + ".d"
STATE = "/var/lib/asterion-agent"
PRIV = "/var/lib/private/asterion-agent"


def steps(center, ca, checksum):
    check = " && ".join(f"test ! -e {path} && test ! -L {path}"
                        for path in (APP, CONF, SYSTEM, SYSTEM + ".d", STATE, PRIV, BOOT, DROP))
    return [
        ("平台", "uname -sm", False),
        ("首次安装检查", f'{check} && test "$(systemctl show -p LoadState --value {UNIT})" = not-found', True),
        ("中心 HTTPS 检查", "curl --disable --noproxy '*' --proto '=https' --fail --silent --show-error --max-time 8 "
         + ("--cacert /dev/stdin " if ca else "") + shlex.quote(center + "/healthz"), False),
        ("创建专用目录", f"mkdir -m 0755 {APP} && mkdir -m 0755 {APP}/releases {APP}/releases/{checksum} {APP}/releases/{checksum}/bin && ln -s releases/{checksum} {APP}/current && mkdir -m 0700 {CONF} {BOOT} && mkdir -m 0755 {DROP}", True),
        ("传输 Agent", f"dd of={APP}/current/bin/asterion-agent conv=excl status=none", True),
        ("核对 Agent", f"printf '%s  %s\\n' '{checksum}' {APP}/current/bin/asterion-agent | sha256sum -c - && chmod 0755 {APP}/current/bin/asterion-agent", True),
        ("中心配置", f"dd of={CONF}/agent.env conv=excl status=none", True),
        *(([("安装公开 CA", f"dd of={CONF}/ca.pem conv=excl status=none", True)]) if ca else []),
        ("一次性注册凭证", f"dd of={BOOT}/token conv=excl status=none", True),
        ("安装服务单元", f"dd of={SYSTEM} conv=excl status=none && chmod 0644 {SYSTEM}", True),
        ("首次启动配置", f"dd of={DROP}/10-enrollment.conf conv=excl status=none && chmod 0644 {DROP}/10-enrollment.conf", True),
        ("启动 Agent", f"systemctl daemon-reload && systemctl enable --now {UNIT}", True),
        ("服务状态", f"systemctl is-active {UNIT}", True),
        ("清理首次启动配置", f"""if grep -Eq '"node_id"[[:space:]]*:[[:space:]]*"[0-9a-f]{{32}}"' {STATE}/identity.json 2>/dev/null; then rm -- {BOOT}/token {DROP}/10-enrollment.conf && rmdir -- {BOOT} {DROP} && systemctl daemon-reload; else printf '%s\\n' pending-enrollment; fi""", True),
    ]


def reinstall_steps(center, ca, checksum, node_id):
    require(re.fullmatch(r"[0-9a-f]{32}", node_id), "节点 ID 无效")
    initial = steps(center, ca, checksum)
    identity = (f"""test -f {STATE}/identity.json && test ! -L {STATE}/identity.json && """
                f"""grep -Eq '"node_id"[[:space:]]*:[[:space:]]*"{node_id}"' {STATE}/identity.json && """
                f"grep -Fxq {shlex.quote('ASTERION_SERVER_URL=' + center)} {CONF}/agent.env")
    # Keep the complete release (including optional helpers) and the previous symlink target.
    # Only change current after the upload has been verified. The trap handles a failed restart.
    install = f"""set -eu
exec 9<{APP}
flock -n 9
{identity} || exit 1
old=$(readlink -e {APP}/current)
test -L {APP}/current
test "$(dirname "$old")" = {APP}/releases
for dir in {APP} {APP}/releases "$old" "$old/bin"; do
    test -d "$dir" && test ! -L "$dir" && test "$(stat -c %u "$dir")" = 0 && test -z "$(find "$dir" -maxdepth 0 -perm /022 -print)" || exit 1
done
stage=$(mktemp -d {APP}/releases/agent-XXXXXXXX)
work=$(mktemp -d {APP}/.switch-XXXXXXXX) || {{ rmdir "$stage"; exit 1; }}
switched=no
cleanup() {{
    result=$?
    trap - EXIT HUP INT TERM
    if test "$result" -ne 0; then
        if test "$switched" = yes; then
            mv -Tf "$work/previous" {APP}/current && systemctl restart {UNIT} || exit 1
        fi
        rm -rf -- "$stage"
    fi
    rm -rf -- "$work"
    exit "$result"
}}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
dd of="$work/agent" conv=excl status=none
printf '%s  %s\\n' '{checksum}' "$work/agent" | sha256sum -c -
chmod 0755 "$work/agent"
if cmp -s "$work/agent" "$old/bin/asterion-agent"; then
    rmdir "$stage"
    systemctl restart {UNIT}
    systemctl is-active --quiet {UNIT}
    exit 0
fi
cp -al -- "$old/." "$stage/"
mv -Tf "$work/agent" "$stage/bin/asterion-agent"
chmod 0755 "$stage"
ln -s "$old" "$work/previous"
ln -s "$stage" "$work/next"
switched=yes
mv -Tf "$work/next" {APP}/current
systemctl restart {UNIT}
systemctl is-active --quiet {UNIT}"""
    return [initial[0], ("核对节点身份与中心", identity, True), initial[2],
            ("传输 Agent", install, True), ("服务状态", f"systemctl is-active {UNIT}", True)]


def invoke(ssh, command, payload=None, transfer_size=0):
    with ssh.open_session(timeout=8) as channel:
        channel.settimeout(20)
        channel.set_combine_stderr(True)
        channel.exec_command(command)
        if payload is not None:
            sent = 0
            last = 0
            while block := payload.read(65536):
                channel.sendall(block)
                sent += len(block)
                if transfer_size:
                    progress = min(100, sent * 100 // transfer_size)
                    if progress >= last + 10 or progress == 100:
                        emit(line=f"Agent 传输：{progress}%")
                        last = progress
        channel.shutdown_write()
        output = bytearray()
        deadline = time.monotonic() + 40
        while True:
            if channel.recv_ready():
                output.extend(channel.recv(4096))
                require(len(output) <= 16384, "远端输出超限，部署已停止")
            elif channel.exit_status_ready():
                break
            elif time.monotonic() >= deadline:
                raise Failure("远端命令超时")
            else:
                time.sleep(.025)
        return channel.recv_exit_status(), bytes(output).decode(errors="replace").strip()


def shell_command(command, privileged, user):
    return ("sudo -n -- " if privileged and user != "root" else "") + "/bin/sh -c " + shlex.quote("umask 077; " + command)


def deploy(ssh, req, plan, binary, ca, center, arch):
    env = f"ASTERION_SERVER_URL={center}\nASTERION_AGENT_STATE_FILE={STATE}/identity.json\n".encode()
    extra = (f"LoadCredential=ca.pem:{CONF}/ca.pem\nEnvironment=ASTERION_AGENT_CA_FILE=%d/ca.pem\n" if ca else "")
    unit = req["unit"].replace("[Service]\n", "[Service]\n" + extra).encode()
    drop = (f"[Service]\nLoadCredential=enrollment:{BOOT}/token\nRuntimeDirectory=asterion-agent-bootstrap\n"
            "RuntimeDirectoryMode=0700\nExecStartPre=/usr/bin/install -m 0600 %d/enrollment /run/asterion-agent-bootstrap/enrollment\n"
            "Environment=ASTERION_AGENT_ENROLLMENT_FILE=/run/asterion-agent-bootstrap/enrollment\n").encode()
    payloads = {"中心配置":env, "一次性注册凭证":(req.get("token") or "").encode(), "安装服务单元":unit, "首次启动配置":drop,
                "安装公开 CA":ca, "中心 HTTPS 检查":ca}
    for title, command, privileged in plan:
        actual = shell_command(command, privileged, req["user"])
        emit(line="$ " + actual)
        with contextlib.ExitStack() as stack:
            payload = stack.enter_context(binary.open("rb")) if title == "传输 Agent" else None
            if data := payloads.get(title):
                payload = io.BytesIO(data)
            status, output = invoke(ssh, actual, payload, binary.stat().st_size if title == "传输 Agent" else 0)
        emit(line=f"{title} · 退出码 {status}")
        if status:
            known = {"sudo: a password is required": "部署用户需要免交互 sudo",
                     "Permission denied": "目标权限不足", "File exists": "目标文件已存在",
                     "certificate": "远端 TLS 证书校验失败", "Could not resolve host": "远端无法解析中心地址"}
            diagnosis = next((label for snippet, label in known.items() if snippet in output), None)
            raise Failure(f"{title}失败：" + (diagnosis or "请核对权限、已有安装、网络或节点服务日志"))
        if title == "平台":
            require(output == "Linux " + arch, "操作系统或 CPU 架构与发布包不匹配")
            emit(line=output)
        if title == "中心 HTTPS 检查":
            try:
                healthy = json.loads(output) == {"status":"ok"}
            except ValueError:
                healthy = False
            require(healthy, "目标机连接到的不是健康的 Asterion 中心")
            emit(line='{"status":"ok"}')
        if title == "服务状态":
            require(output == "active", "Agent 服务未进入 active 状态")
            emit(line="active")
        if title == "清理首次启动配置" and output == "pending-enrollment":
            emit(line="Agent 尚未保存注册身份；保留本次启动凭据以便重试（重启主机即清除）")
        if title == "核对 Agent" and output == f"{APP}/current/bin/asterion-agent: OK":
            emit(line=output)


def run(req):
    import paramiko
    center = origin(os.environ["ASTERION_SSH_DEPLOY_CENTER_URL"])
    binary = Path(os.environ["ASTERION_SSH_DEPLOY_AGENT"])
    require(binary.is_absolute() and binary.is_file() and 0 < binary.stat().st_size <= 256 * 1024**2,
            "中心未配置可部署的 Agent 二进制文件")
    arch = arch_of(binary)
    ca = None
    if path := os.environ.get("ASTERION_SSH_DEPLOY_CA_FILE"):
        with Path(path).open("rb") as stream:
            ca = stream.read(65537)
        require(0 < len(ca) <= 65536 and b"BEGIN CERTIFICATE" in ca and b"PRIVATE KEY" not in ca, "部署 CA 必须是公开证书")
    host, port = req["host"], req["port"]
    address = destination(host, port, os.environ["ASTERION_SSH_DEPLOY_NETWORKS"], req.get("address"))
    checksum = hashlib.sha256()
    with binary.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            checksum.update(chunk)
    reinstall = req.get("reinstall")
    plan = (reinstall_steps(center, ca, checksum.hexdigest(), reinstall) if reinstall
            else steps(center, ca, checksum.hexdigest()))
    manifest = hashlib.sha256(json.dumps([plan, hashlib.sha256(ca or b"").hexdigest(), req["unit"]]).encode()).hexdigest()
    with socket.create_connection((address, port), timeout=4) as sock, contextlib.closing(paramiko.Transport(sock)) as ssh:
        ssh.banner_timeout = 5
        ssh.auth_timeout = 10
        ssh.start_client(timeout=5)
        identity = fingerprint(ssh.get_remote_server_key())
        if req["mode"] == "inspect":
            emit(address=address, fingerprint=identity, center=center, manifest=manifest,
                 commands=[shell_command(cmd, privileged, req["user"]) for _, cmd, privileged in plan])
            return
        require(hmac.compare_digest(identity, req["fingerprint"]), "SSH 主机指纹已改变，未发送登录凭据")
        require(hmac.compare_digest(manifest, req["manifest"]), "发布产物或配置已变化，请重新核对")
        authenticate(ssh, req, paramiko)
        emit(line="SSH 身份验证成功")
        deploy(ssh, req, plan, binary, ca, center, arch)
    emit(line="SSH 会话已关闭")


if __name__ == "__main__":
    try:
        run(json.load(sys.stdin))
    except Failure as error:
        emit(error=str(error))
        sys.exit(1)
    except ImportError:
        emit(error="中心缺少 python3-paramiko，无法通过 SSH 部署")
        sys.exit(1)
    except Exception as exc:
        if type(exc).__name__ in ("AuthenticationException", "BadAuthenticationType", "PartialAuthentication"):
            emit(error="SSH 认证失败，请检查用户名、凭据及目标机允许的认证方式")
            sys.exit(1)
        emit(error="SSH 连接、认证、传输或中心发布配置异常；未输出可能含凭据的原始异常")
        sys.exit(1)
