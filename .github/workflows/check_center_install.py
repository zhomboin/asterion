"""在一次性 GitHub Ubuntu 虚拟机上，从手动账号和半解包目录完成安装并重跑。"""

import hashlib
import os
import secrets
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/center"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import host
import install
import pki
import services


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def make_archive(work):
    release = work / "release"
    (release / "bin").mkdir(parents=True)
    for name in ("asterion-server",):
        run("strip", "-o", release / "bin" / name, ROOT / "target/release" / name)
    (release / "web").mkdir()
    (release / "web/index.html").write_text("Asterion")
    (release / "deploy").mkdir()
    for name in (host.SERVER,):
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


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.geteuid() != 0:
        sys.exit("此测试只在一次性 GitHub Actions 虚拟机运行。")
    os.umask(0o022)
    if any(
        os.path.lexists(p)
        for p in (
            install.BASE,
            install.CONFIG,
        )
    ):
        sys.exit("测试机已有 Asterion")
    ops = "asterion-ci-" + secrets.token_hex(3)
    domain = "panel.example.invalid"
    prompt = {
        "port": "18080",
        "ops": ops,
        "domain": domain,
        "node": "",
    }
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        archive = make_archive(work)
        original_run = host.run

        def local_commands(*args, **kwargs):
            if args[0] == "certbot":
                return original_run(
                    *args, "--server", "https://localhost:14000/dir", **kwargs
                )
            return original_run(*args, **kwargs)

        with (
            patch.object(install, "settings", return_value=prompt),
            patch.object(host, "run", side_effect=local_commands),
            patch.object(host, "ask", return_value="admin@example.invalid") as email,
            patch.object(
                services, "password", return_value=secrets.token_hex(24)
            ) as password,
            patch.object(
                pki, "ca_password", return_value=secrets.token_hex(24) + "\n"
            ) as ca,
        ):
            install.provision(archive)

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

            before = [pid(unit) for unit in (host.SERVER,)]

            def status(path, source):
                return run(
                    "curl",
                    "-sS",
                    "--noproxy",
                    "*",
                    "--interface",
                    source,
                    "--resolve",
                    f"{domain}:443:127.0.0.1",
                    "-o",
                    os.devnull,
                    "-w",
                    "%{http_code}",
                    f"https://{domain}{path}",
                    capture_output=True,
                ).stdout

            def wait_access(expected):
                for _ in range(25):
                    if status("/api/v1/me", "127.0.0.2") == expected:
                        return
                    time.sleep(0.2)
                raise AssertionError(f"管理 API 未返回 {expected!r}")

            # 从有标准 IP 限制的手动站点续装，确认动态地址仍需要登录。
            host.replace_text(
                host.site_path(prompt),
                host.site_config(prompt, allowed=["127.0.0.1", "192.0.2.1"]),
            )
            run("systemctl", "reload", "nginx")
            wait_access(b"403")
            install.STATE.unlink()
            install.DONE.unlink()
            install.provision(archive)
            wait_access(b"401")
            attempts = [status("/api/v1/auth/login", "127.0.0.2") for _ in range(8)]
            assert attempts[0] == b"405" and b"429" in attempts
            assert status("/api/v1/auth/login", "127.0.0.3") == b"405"
            assert status("/healthz", "127.0.0.2") == b"200"
            local_commands("certbot", "renew", "--cert-name", domain, "--dry-run")
            assert [pid(unit) for unit in (host.SERVER,)] == before
            password.assert_called_once()
            ca.assert_called_once()
            email.assert_called_once()


if __name__ == "__main__":
    main()
