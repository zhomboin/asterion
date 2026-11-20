"""在一次性 GitHub Ubuntu 虚拟机上，从手动账号和半解包目录完成安装并重跑。"""

import grp
import hashlib
import io
import json
import os
import pwd
import secrets
import shlex
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
import docker_reader
import host
import install
import pki
import services


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def make_archive(work):
    release = work / "release"
    (release / "bin").mkdir(parents=True)
    for name in ("asterion-server", "asterion-agent", "asterion-docker-reader"):
        run("strip", "-o", release / "bin" / name, ROOT / "target/release" / name)
    (release / "web").mkdir()
    (release / "web/index.html").write_text("Asterion")
    (release / "deploy").mkdir()
    for name in (host.SERVER, host.AGENT, docker_reader.UNIT, "self_monitor.py"):
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


def check_panel_inventory(work, port, username, password, node_id):
    # 导入本地空镜像并创建一个停止状态的容器，避免拉取镜像或运行业务负载。
    layer = io.BytesIO()
    with tarfile.open(fileobj=layer, mode="w") as bundle:
        bundle.addfile(tarfile.TarInfo("placeholder"))
    image = (
        run("docker", "import", "-", input=layer.getvalue(), capture_output=True)
        .stdout.decode()
        .strip()
    )
    try:
        container = (
            run(
                "docker",
                "create",
                "--network",
                "none",
                image,
                "/unused",
                capture_output=True,
            )
            .stdout.decode()
            .strip()
        )
        try:
            base = f"https://127.0.0.1:{port}/api/v1"
            curl = [
                "curl",
                "--noproxy",
                "*",
                "--silent",
                "--show-error",
                "--fail",
                "--cacert",
                install.CONFIG / "deploy-ca.crt",
            ]
            cookie = work / "docker-test.cookies"
            run(
                *curl,
                "-c",
                cookie,
                "-H",
                "X-Asterion-CSRF: 1",
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
                base + "/auth/login",
                input=json.dumps({"username": username, "password": password}).encode(),
                stdout=subprocess.DEVNULL,
            )
            result = run(
                *curl,
                "-b",
                cookie,
                "--retry",
                "8",
                "--retry-delay",
                "1",
                f"{base}/nodes/{node_id}/docker/containers",
                capture_output=True,
            )
            data = json.loads(result.stdout)
            assert data["status"] == "available" and data["engine_id"]
            assert any(row["id"] == container for row in data["containers"])
        finally:
            run("docker", "rm", container)
    finally:
        run("docker", "image", "rm", image)


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
        "node": "",
    }
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
            if args[0] == "certbot":
                return original_run(
                    *args, "--server", "https://localhost:14000/dir", **kwargs
                )
            return original_run(*args, **kwargs)

        with (
            patch.object(install, "settings", return_value=prompt),
            patch.object(docker_reader, "ask", return_value="2") as docker_choice,
            patch.object(host, "run", side_effect=local_commands),
            patch.object(host, "ask", return_value="admin@example.invalid") as email,
            patch.object(
                services, "password", return_value=secrets.token_hex(24)
            ) as password,
            patch.object(
                pki, "ca_password", return_value=secrets.token_hex(24) + "\n"
            ) as ca,
        ):
            try:
                install.provision(archive)
                assert not services.DOCKER_CONF.exists()
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
                email.assert_called_once()
                run("systemctl", "start", "docker.service")
                assert docker_reader.detect()[0] == "ready"
                # 从实际的一键入口补配。工作目录没有归档，无需重打包或更新发布链接。
                run(
                    "script",
                    "-qec",
                    shlex.join(["bash", str(ROOT / "deploy/install-center.sh")]),
                    "/dev/null",
                    input=b"1\n",
                    cwd=work.parent,
                )
                after = [
                    pid(unit) for unit in (host.SERVER, host.AGENT, docker_reader.UNIT)
                ]
                assert after[0] == before[0] and after[1] != before[1]
                assert services.IDENTITY.read_bytes() == identity
                docker_gid = grp.getgrnam("docker").gr_gid
                assert docker_gid not in os.getgrouplist(ops, account.pw_gid)
                access = docker_reader.agent_access(
                    grp.getgrnam(docker_reader.GROUP).gr_gid
                )
                assert access and docker_gid not in (
                    access["group"],
                    *access["extra_groups"],
                )
                check_panel_inventory(
                    work,
                    prompt["port"],
                    ops + "-panel",
                    password.return_value,
                    json.loads(identity)["node_id"],
                )
                docker_choice.return_value = "1"
                install.provision(archive)
                assert docker_choice.call_args.args[-1] == "2"
                assert [
                    pid(unit) for unit in (host.SERVER, host.AGENT, docker_reader.UNIT)
                ] == after
            finally:
                subprocess.run(
                    [
                        "systemctl",
                        "stop",
                        host.AGENT,
                        host.SERVER,
                        docker_reader.UNIT,
                        "certbot.timer",
                        "nginx",
                    ],
                    check=False,
                )


if __name__ == "__main__":
    main()
