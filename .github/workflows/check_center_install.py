"""在一次性 GitHub Ubuntu 虚拟机上，从手动账号和半解包目录完成安装并重跑。"""

import hashlib
import json
import os
import pwd
import secrets
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/center"))
import host
import install
import pki
import services


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def make_archive(work):
    release = work / "release"
    (release / "bin").mkdir(parents=True)
    for name in ("asterion-server", "asterion-agent"):
        run("strip", "-o", release / "bin" / name, ROOT / "target/release" / name)
    (release / "web").mkdir()
    (release / "web/index.html").write_text("Asterion")
    (release / "deploy").mkdir()
    for name in (host.SERVER, host.AGENT, "self_monitor.py"):
        shutil.copyfile(ROOT / "deploy" / name, release / "deploy" / name)
    unit = release / "deploy" / host.SERVER
    unit.write_text(unit.read_text().replace("/run/credentials/%n/", "%d/"))
    host.put(host.UNITS / host.SERVER, unit.read_text(), 0o644)
    archive = work / "asterion-ci.tar.gz"
    with tarfile.open(archive, "w:gz", compresslevel=1) as bundle:
        for directory in release.iterdir():
            bundle.add(directory, arcname=directory.name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    Path(str(archive) + ".sha256").write_text(f"{digest}  {archive.name}\n")
    # 手动解包中断时可能已有部分文件，但没有安装器的 .sha256。
    partial = install.BASE / "releases/ci/web"
    # 手动部署命令为发布树明确设置 0755，避免继承 CI 工具目录的组写权限。
    run(
        "install",
        "-d",
        "-m",
        "0755",
        install.BASE,
        install.BASE / "releases",
        partial.parent,
        partial,
    )
    shutil.copyfile(release / "web/index.html", partial / "index.html")
    (partial / "index.html").chmod(0o644)
    return archive


def public_certificate(domain):
    cert = host.LE / "live" / domain
    cert.mkdir(parents=True)
    run(
        "openssl",
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "1",
        "-subj",
        "/CN=" + domain,
        "-addext",
        "subjectAltName=DNS:" + domain,
        "-addext",
        "basicConstraints=critical,CA:TRUE",
        "-keyout",
        cert / "privkey.pem",
        "-out",
        cert / "fullchain.pem",
    )
    host.put(host.LE / "renewal" / (domain + ".conf"), "# 本机测试证书\n")
    shutil.copyfile(
        cert / "fullchain.pem", "/usr/local/share/ca-certificates/asterion-ci.crt"
    )
    run("update-ca-certificates")


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.geteuid() != 0:
        sys.exit("此测试只在一次性 GitHub Actions 虚拟机运行。")
    os.umask(0o022)
    if any(
        os.path.lexists(p)
        for p in (
            install.BASE,
            install.CONFIG,
            install.AGENT_BASE,
            install.AGENT_CONFIG,
        )
    ):
        sys.exit("测试机已有 Asterion")
    ops = "asterion-ci-" + secrets.token_hex(3)
    domain = "panel.example.invalid"
    run("adduser", "--disabled-password", "--gecos", "", ops)
    account = pwd.getpwnam(ops)
    keys = Path(account.pw_dir) / ".ssh/authorized_keys"
    run("runuser", "-u", ops, "--", "mkdir", "-p", "-m", "700", keys.parent)
    prompt = {
        "port": "18080",
        "ops": ops,
        "domain": domain,
        "admin_ip": "192.0.2.1",
        "node": "",
        "agent": "",
    }
    public_certificate(domain)
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", work / "key")
        key = (work / "key.pub").read_bytes()
        run(
            "runuser",
            "-u",
            ops,
            "--",
            "sh",
            "-c",
            'umask 077; cat > "$1"',
            "sh",
            keys,
            input=key,
        )
        archive = make_archive(work)
        original_run = host.run

        def local_commands(*args, **kwargs):
            if args[:2] == ("certbot", "renew"):
                assert args == ("certbot", "renew", "--cert-name", domain, "--dry-run")
                print("公网 ACME 验证在隔离环境中跳过。")
                return None
            return original_run(*args, **kwargs)

        with (
            patch.object(install, "settings", return_value=prompt),
            patch.object(host, "run", side_effect=local_commands),
            patch.object(
                services, "password", return_value=secrets.token_hex(24)
            ) as password,
            patch.object(
                pki, "ca_password", return_value=secrets.token_hex(24) + "\n"
            ) as ca,
        ):
            try:
                install.provision(archive)
                identity = services.IDENTITY.read_bytes()

                def pid(unit):
                    return run(
                        "systemctl",
                        "show",
                        "--value",
                        "-p",
                        "MainPID",
                        unit,
                        capture_output=True,
                    ).stdout

                before = [pid(unit) for unit in (host.SERVER, host.AGENT)]
                # 模拟手动部署到服务已上线，只有安装器自己的进度文件缺失。
                install.STATE.unlink()
                install.DONE.unlink()
                install.provision(archive)
                assert [pid(unit) for unit in (host.SERVER, host.AGENT)] == before
                assert (
                    services.IDENTITY.read_bytes() == identity
                    and json.loads(identity)["node_id"]
                )
                assert (
                    pwd.getpwnam(ops).pw_uid == account.pw_uid
                    and keys.read_bytes() == key
                )
                ca.assert_called_once()
                password.assert_called_once()
            finally:
                subprocess.run(
                    [
                        "systemctl",
                        "stop",
                        host.AGENT,
                        host.SERVER,
                        "certbot.timer",
                        "nginx",
                    ],
                    check=False,
                )


if __name__ == "__main__":
    main()
