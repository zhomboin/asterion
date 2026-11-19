"""安装中心主机的配置、证书、运维账号与 HTTPS 入口。"""

import os
import pwd
import re
import subprocess
import tempfile
from pathlib import Path

from inputs import ask

HERE = Path(__file__).resolve().parent
CONFIG = Path("/etc/asterion")
LE = Path("/etc/letsencrypt")
UNITS = Path("/etc/systemd/system")
RUNTIME = Path("/run/systemd/system")
SERVER = "asterion-server.service"
AGENT = "asterion-agent.service"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def put(path, data, mode=0o600):
    """原子发布新文件。重试只接受同内容的普通文件。"""
    if path.is_symlink():
        raise ValueError(f"已有链接，请检查 {path}")
    if path.exists():
        if (
            path.is_file()
            and path.read_text() == data
            and path.stat().st_mode & 0o777 == mode
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


def internal_tls(directory, passphrase):
    pki = directory / "pki"
    pki.mkdir(mode=0o700)
    secret = passphrase + "\n"
    run(
        "openssl",
        "genrsa",
        "-aes256",
        "-passout",
        "stdin",
        "-out",
        "pki/ca.key",
        "3072",
        cwd=directory,
        input=secret,
    )
    run(
        "openssl",
        "req",
        "-x509",
        "-new",
        "-sha256",
        "-days",
        "1825",
        "-key",
        "pki/ca.key",
        "-passin",
        "stdin",
        "-subj",
        "/CN=Asterion Internal CA",
        "-addext",
        "basicConstraints=critical,CA:TRUE,pathlen:0",
        "-addext",
        "keyUsage=critical,keyCertSign,cRLSign",
        "-out",
        "deploy-ca.crt",
        cwd=directory,
        input=secret,
    )
    run(
        "openssl",
        "req",
        "-new",
        "-newkey",
        "rsa:3072",
        "-nodes",
        "-subj",
        "/CN=localhost",
        "-keyout",
        "tls.key",
        "-out",
        "tls.csr",
        cwd=directory,
    )
    put(
        directory / "tls.ext",
        "basicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n"
        "keyUsage=critical,digitalSignature,keyEncipherment\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n",
    )
    run(
        "openssl",
        "x509",
        "-req",
        "-sha256",
        "-days",
        "180",
        "-in",
        "tls.csr",
        "-CA",
        "deploy-ca.crt",
        "-CAkey",
        "pki/ca.key",
        "-passin",
        "stdin",
        "-CAserial",
        "pki/ca.srl",
        "-CAcreateserial",
        "-extfile",
        "tls.ext",
        "-out",
        "tls.crt",
        cwd=directory,
        input=secret,
    )
    for option, host in [
        ("-verify_hostname", "localhost"),
        ("-verify_ip", "127.0.0.1"),
    ]:
        run(
            "openssl",
            "verify",
            "-purpose",
            "sslserver",
            option,
            host,
            "-CAfile",
            "deploy-ca.crt",
            "tls.crt",
            cwd=directory,
        )
    (directory / "tls.ext").unlink()
    (directory / "tls.csr").unlink()


def operator(c, work):
    name = c["ops"]
    marker = "Asterion center operator"
    try:
        account = pwd.getpwnam(name)
    except KeyError:
        run("adduser", "--disabled-password", "--gecos", marker, name)
        account = pwd.getpwnam(name)
    if account.pw_gecos.split(",")[0] != marker or account.pw_uid == 0:
        raise ValueError("运维账号已被其他用途占用")
    ssh = Path(account.pw_dir) / ".ssh"
    as_user = ["runuser", "-u", name, "--"]
    run(*as_user, "mkdir", "-p", "-m", "0700", ssh)
    keyfile = ssh / "authorized_keys"
    run(
        *as_user,
        "sh",
        "-c",
        'if test -e "$1"; then cmp -s - "$1"; else umask 077; set -C; cat > "$1"; fi',
        "sh",
        keyfile,
        input=c["key"] + "\n",
    )
    rule = (
        f"{name} ALL=(root) NOPASSWD: /usr/bin/systemctl restart {SERVER}, "
        f"/usr/bin/journalctl --no-pager -u {SERVER} -n 100\n"
    )
    put(work / "sudoers", rule, 0o440)
    run("visudo", "-cf", work / "sudoers")
    put(Path("/etc/sudoers.d") / f"asterion-{name}", rule, 0o440)


def site_path(c):
    return Path("/etc/nginx/conf.d") / (c["domain"] + ".conf")


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


def public_tls(c):
    domain = c["domain"]
    cert = LE / "live" / domain
    credentials = LE / "asterion" / "cloudflare.ini"
    if not (LE / "renewal" / f"{domain}.conf").is_file():
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
    put(LE / "renewal-hooks/deploy" / f"asterion-{domain}", hook, 0o755)
    run("systemctl", "enable", "--now", "certbot.timer")
    run("certbot", "renew", "--cert-name", domain, "--dry-run", "--run-deploy-hooks")


def enable_site(c):
    site = site_path(c)
    new = not site.exists()
    if new:
        check_site(c)
    agent_allow = (
        f"    allow {c['agent']};" if c["agent"] and c["agent"] != c["admin_ip"] else ""
    )
    put(
        site,
        template(
            "nginx.conf",
            domain=c["domain"],
            port=c["port"],
            admin_ip=c["admin_ip"],
            agent_allow=agent_allow,
        ),
        0o644,
    )
    try:
        run("nginx", "-t")
        run("systemctl", "reload", "nginx")
    except subprocess.CalledProcessError:
        if new:
            site.unlink()
        raise
