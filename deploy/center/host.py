"""安装中心主机的配置、证书、运维账号与 HTTPS 入口。"""

import os
import re
import subprocess
import tempfile
from pathlib import Path

from existing import authorized_keys, login_account, trusted
from inputs import ask, public_key

HERE = Path(__file__).resolve().parent
CONFIG = Path("/etc/asterion")
LE = Path("/etc/letsencrypt")
UNITS = Path("/etc/systemd/system")
RUNTIME = Path("/run/systemd/system")
SUDOERS = Path("/etc/sudoers.d")
SITES = Path("/etc/nginx/conf.d")
SERVER = "asterion-server.service"
AGENT = "asterion-agent.service"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def put(path, data, mode=0o600):
    """原子发布新文件。重试只接受同内容的普通文件。"""
    if path.is_symlink():
        raise ValueError(f"已有链接，请检查 {path}")
    if path.exists():
        info = path.stat()
        permissions = info.st_mode & 0o7777
        if (
            path.is_file()
            and info.st_uid == os.geteuid()
            and permissions & ~mode == 0
            and permissions & 0o400
            and permissions & 0o111 == mode & 0o111
            and path.read_text() == data
        ):
            return
        raise ValueError(f"已有配置内容或权限不同，请检查 {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        dir=path.parent, mode="w", encoding="utf-8"
    ) as out:
        out.write(data)
        out.flush()
        os.fchmod(out.fileno(), mode)
        os.link(out.name, path)


def template(name, **values):
    data = (HERE / (name + ".in")).read_text()
    for key, value in values.items():
        data = data.replace(f"@{key}@", value)
    return data


def server_config(c):
    lines = [
        f"ASTERION_LISTEN_ADDR=127.0.0.1:{c['port']}",
        "ASTERION_DATABASE_URL=sqlite:///var/lib/asterion/asterion.db",
        "ASTERION_WEB_DIR=/opt/asterion/current/web",
        "ASTERION_TERMINAL_ENABLED=0",
    ]
    if c["node"]:
        lines.extend(
            [
                "ASTERION_SSH_DEPLOY_AGENT=/opt/asterion/current/bin/asterion-agent",
                f"ASTERION_SSH_DEPLOY_CENTER_URL=https://{c['domain']}",
                f"ASTERION_SSH_DEPLOY_NETWORKS={c['node']}/32",
            ]
        )
    return "\n".join(lines) + "\n"


def operator(c, work, setup=None):
    name = c["ops"]
    account = login_account(name)
    if account is None:
        if os.path.lexists(Path("/home") / name):
            raise ValueError(f"已有同名家目录但找不到账号：{name}")
        run(
            "adduser",
            "--disabled-password",
            "--gecos",
            "Asterion center operator",
            name,
        )
        account = login_account(name)
    else:
        print(f"复用运维账号 {name}。")
    if not authorized_keys(account):
        key = c.get("key")
        if (
            not key
            and setup
            and trusted(setup, directory=True, private=True)
            and trusted(setup / "ops-key.pub")
        ):
            key = (setup / "ops-key.pub").read_text().strip()
            if len(key.splitlines()) != 1 or not re.fullmatch(
                r"(?:ssh-|ecdsa-|sk-)[\w@.-]+ [A-Za-z0-9+/=]+(?: [^\r\n]*)?", key
            ):
                raise ValueError("手动暂存的运维公钥格式无效")
            run("ssh-keygen", "-lf", "-", input=key + "\n")
        key = key or public_key()
        ssh = Path(account.pw_dir) / ".ssh"
        as_user = ["runuser", "-u", name, "--"]
        run(*as_user, "mkdir", "-p", "-m", "0700", ssh)
        run(
            *as_user,
            "sh",
            "-c",
            'set -e; umask 077; test ! -L "$1"; if test -e "$1"; then test -f "$1"; test ! -s "$1"; else set -C; fi; cat > "$1"',
            "sh",
            ssh / "authorized_keys",
            input=key + "\n",
        )
    rule = (
        f"{name} ALL=(root) NOPASSWD: /usr/bin/systemctl restart {SERVER}, "
        f"/usr/bin/journalctl --no-pager -u {SERVER} -n 100\n"
    )
    put(work / "sudoers", rule, 0o440)
    run("visudo", "-cf", work / "sudoers")
    destination = SUDOERS / f"asterion-{name}"
    if trusted(destination) and destination.read_text() != rule:
        run("visudo", "-cf", destination)
        for command in (
            ["/usr/bin/systemctl", "restart", SERVER],
            ["/usr/bin/journalctl", "--no-pager", "-u", SERVER, "-n", "100"],
        ):
            run("sudo", "-n", "-l", "-U", name, *command, stdout=subprocess.DEVNULL)
        return
    put(destination, rule, 0o440)


def site_path(c):
    return SITES / (c["domain"] + ".conf")


def check_site(c):
    config = run("nginx", "-T", stdout=subprocess.PIPE).stdout
    config = re.sub(r"#[^\n]*", "", config)
    for names in re.findall(r"\bserver_name\s+([^;{}]+);", config):
        if c["domain"] in [s.strip("'\"").lower() for s in names.split()]:
            raise ValueError("Nginx 已有同域名站点，请使用部署文档的已有站点说明")
    if not re.search(
        r'\binclude\s+["\']?/etc/nginx/conf\.d/\*\.conf["\']?\s*;', config
    ):
        raise ValueError("Nginx 未加载 /etc/nginx/conf.d/*.conf")


def public_tls(c, verified=False):
    domain = c["domain"]
    cert = LE / "live" / domain
    credentials = LE / "asterion" / "cloudflare.ini"
    if not (LE / "renewal" / f"{domain}.conf").is_file():
        verified = False
        email = ask(
            "Let's Encrypt 注册邮箱",
            lambda s: re.fullmatch(r"[^@\s]+@[^@\s]+\.[^@\s]+", s),
        )
        new_credentials = not credentials.exists()
        if new_credentials:
            token = ask(
                "Cloudflare 区域 DNS 编辑令牌",
                lambda s: re.fullmatch(r"[A-Za-z0-9_-]+", s),
                secret=True,
            )
            credentials.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            put(credentials, f"dns_cloudflare_api_token = {token}\n")
        if (
            credentials.is_symlink()
            or credentials.stat().st_uid != 0
            or credentials.stat().st_mode & 0o077
        ):
            raise ValueError(f"请检查 DNS 令牌属主和权限 {credentials}")
        try:
            run(
                "certbot",
                "certonly",
                "--non-interactive",
                "--agree-tos",
                "--email",
                email,
                "--cert-name",
                domain,
                "-d",
                domain,
                "--dns-cloudflare",
                "--dns-cloudflare-credentials",
                credentials,
                "--dns-cloudflare-propagation-seconds",
                "60",
            )
        except subprocess.CalledProcessError:
            if new_credentials and not (LE / "renewal" / f"{domain}.conf").exists():
                credentials.unlink()
            raise
    run(
        "openssl",
        "verify",
        "-purpose",
        "sslserver",
        "-verify_hostname",
        domain,
        "-CAfile",
        "/etc/ssl/certs/ca-certificates.crt",
        "-untrusted",
        cert / "fullchain.pem",
        cert / "fullchain.pem",
    )
    hook = (
        f'#!/bin/sh\n[ "$RENEWED_LINEAGE" = "{cert}" ] || exit 0\n'
        "set -e\n/usr/sbin/nginx -t\n/usr/bin/systemctl reload nginx\n"
    )
    renewal = LE / "renewal" / f"{domain}.conf"
    inline_hook = re.search(
        r"(?m)^\s*(?:renew_hook|deploy_hook)\s*=\s*/usr/sbin/nginx -t && /usr/bin/systemctl reload nginx\s*$",
        renewal.read_text(),
    )
    if not inline_hook:
        path = LE / "renewal-hooks/deploy" / f"asterion-{domain}"
        verified = verified and path.is_file()
        put(path, hook, 0o755)
    run("systemctl", "enable", "--now", "certbot.timer")
    # 两个发行版均支持 dry-run；分别验证 Nginx 配置，正式续期仍执行 deploy hook。
    if not verified:
        run("certbot", "renew", "--cert-name", domain, "--dry-run")
    run("nginx", "-t")


def site_config(c):
    agent_allow = (
        f"    allow {c['agent']};" if c["agent"] and c["agent"] != c["admin_ip"] else ""
    )
    return template(
        "nginx.conf",
        domain=c["domain"],
        port=c["port"],
        admin_ip=c["admin_ip"],
        agent_allow=agent_allow,
    )


def check_existing_site(c):
    site = site_path(c)
    if not trusted(site):
        check_site(c)
        return False

    def directives(data):
        result = []
        for line in data.splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                if line.startswith("allow ") and result[-1:] == [line]:
                    continue
                result.append(line)
        return result

    if directives(site.read_text()) != directives(site_config(c)):
        raise ValueError(f"已有 Nginx 站点与目标代理或访问范围不一致，已保留：{site}")
    return True


def enable_site(c, live=None):
    site = site_path(c)
    new = not check_existing_site(c)
    if new:
        put(site, site_config(c), 0o644)
    elif live and live():
        print("复用已生效的 Nginx 站点。")
        return
    try:
        run("nginx", "-t")
        run("systemctl", "reload", "nginx")
    except subprocess.CalledProcessError:
        if new:
            site.unlink()
        raise
