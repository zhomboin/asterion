"""使用普通用户和临时 OpenSSH 服务验证面板，无需系统安装。"""

import base64
from contextlib import ExitStack
import json
import os
from pathlib import Path
import pwd
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.request
import urllib.error

from local_panel_test import check_local_panel
from collector_ssh_test import check_collector
from collector_deployment_test import check_deployment


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait(check, message):
    end = time.monotonic() + 18
    while time.monotonic() < end:
        if check():
            return
        time.sleep(0.15)
    raise AssertionError(message)


def launch(stack, command, log, env=None):
    handle = stack.enter_context(log.open("w"))
    process = subprocess.Popen(command, stdout=handle, stderr=handle, env=env)

    def close():
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()

    stack.callback(close)
    return process


def exercise(work, stack):
    system_deploy = os.environ.get("TEST_SYSTEM_DEPLOY") == "1"
    if system_deploy and os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("系统级安装测试仅在专用 GitHub 执行机运行")
    ssh_port = port()
    for key in ("host", "key"):
        subprocess.run(
            ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", work / key],
            check=True,
        )
    for path, key in (("known_hosts", "host"), ("wrong_hosts", "key")):
        (work / path).write_text(
            f"[127.0.0.1]:{ssh_port} " + (work / (key + ".pub")).read_text()
        )
    user = pwd.getpwuid(os.getuid()).pw_name
    (work / "sshd.conf").write_text(f"""Port {ssh_port}
ListenAddress 127.0.0.1
HostKey {work}/host
AuthorizedKeysFile {work}/key.pub
PidFile {work}/sshd.pid
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM {"yes" if system_deploy else "no"}
AllowUsers {user}
DisableForwarding yes
SetEnv HOME={work} PATH={work}/bin:/usr/local/bin:/usr/bin:/bin {"" if system_deploy else f"ASTERION_COLLECTOR_DB={work}/collector.db"}
""")
    system_unit = "asterion-sshd-" + secrets.token_hex(6)
    service = (
        [
            "sudo",
            "systemd-run",
            "--quiet",
            "--wait",
            "--pipe",
            "--collect",
            "--unit",
            system_unit,
        ]
        if system_deploy
        else []
    )
    sshd = launch(
        stack,
        [
            *service,
            os.environ.get("TEST_SSHD", "/usr/sbin/sshd"),
            "-D",
            "-e",
            "-f",
            work / "sshd.conf",
        ],
        work / "sshd.log",
    )

    if system_deploy:
        stack.callback(subprocess.run, ["sudo", "systemctl", "stop", system_unit])
        stack.callback(
            subprocess.run, ["sudo", "systemctl", "stop", "asterion-collector.service"]
        )

    def listening():
        if sshd.poll() is not None:
            raise RuntimeError((work / "sshd.log").read_text())
        try:
            with socket.create_connection(("127.0.0.1", ssh_port), timeout=0.2):
                return True
        except OSError:
            return False

    wait(listening, "SSH 启动失败")
    target = dict(
        host="127.0.0.1",
        port=ssh_port,
        user=user,
        identity_file="key",
        policy=dict(
            commands=[
                dict(name="identity", program="/usr/bin/id", args=["-un"]),
                dict(name="nonzero", program="/usr/bin/false"),
            ],
            managed=dict(packages=dict(shell=dict(package="bash"))),
        ),
    )
    (work / "profiles.json").write_text(
        json.dumps(
            {"good": target, "wrong": {**target, "known_hosts_file": "wrong_hosts"}}
        )
    )
    panel_port = port()
    password = secrets.token_hex(16)
    (work / "password").write_text(password)
    (work / "password").chmod(0o600)
    env = {
        **os.environ,
        "ASTERION_LOCAL": "1",
        "ASTERION_LISTEN_ADDR": f"127.0.0.1:{panel_port}",
        "ASTERION_ADMIN_USERNAME": "fixture",
        "ASTERION_ADMIN_PASSWORD_FILE": str(work / "password"),
        "ASTERION_DATABASE_URL": "sqlite://" + str(work / "db"),
        "ASTERION_SAMPLE_INTERVAL_SECS": "5",
        "ASTERION_TERMINAL_ENABLED": "1",
        "ASTERION_SSH_CONFIG_FILE": str(work / "profiles.json"),
    }
    center = launch(
        stack,
        [os.environ.get("TEST_CENTER", "target/debug/asterion-server")],
        work / "center.log",
        env,
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(path, data=None):
        req = urllib.request.Request(
            f"http://127.0.0.1:{panel_port}" + path,
            data=json.dumps(data).encode() if data is not None else None,
            headers={"X-Asterion-CSRF": "1", "Content-Type": "application/json"},
        )
        with opener.open(req, timeout=12) as reply:
            if path == "/api/v1/auth/login":
                opener.addheaders = [
                    (
                        "X-Asterion-Local-Session",
                        reply.headers["X-Asterion-Local-Session"],
                    )
                ]
            content = reply.read()
            return json.loads(content) if content else None

    def healthy():
        if center.poll() is not None:
            raise RuntimeError((work / "center.log").read_text())
        try:
            return api("/healthz")["status"] == "ok"
        except urllib.error.URLError:
            return False

    wait(healthy, "中心启动失败")
    api("/api/v1/auth/login", dict(username="fixture", password=password))
    assert api("/api/v1/ssh/profiles") == ["good", "wrong"]

    def create(profile):
        node = api(
            "/api/v1/nodes", dict(name=profile, kind="linux", ssh_profile=profile)
        )
        return "/api/v1/nodes/" + node["id"]

    node, wrong = create("good"), create("wrong")
    try:
        create("ssh://-invalid")
    except urllib.error.HTTPError as error:
        assert error.code == 400
    else:
        raise AssertionError("面板接受了无效 SSH 地址")
    wait(lambda: api(wrong)["status"] == "offline", "主机密钥不匹配必须离线")
    try:
        wait(lambda: api(node)["status"] == "online", "正常 SSH 节点应上线")
    except AssertionError:
        print(api(node)["ssh_error"], (work / "sshd.log").read_text())
        raise
    assert api(node + "/metrics")["samples"][-1]["sample"]["memory"]["total_bytes"] > 0
    deploy_checks = os.environ.get("TEST_DEPLOY_CHECKS") == "1"
    if system_deploy or deploy_checks:
        check_deployment(api, node, password, work, system_deploy, target)
        if system_deploy:
            return
    if deploy_checks or os.environ.get("TEST_COLLECTOR_ONLY"):
        check_collector(work, stack, sshd, node, api, launch, wait, target)
        return
    assert api(node + "/processes?limit=2")
    assert api(node + "/managed/packages?key=shell")["installed"]
    assert api(node + "/accounts")["users"]
    assert api(node + "/systemd")["status"]
    assert api(node + "/docker")["status"]
    for name, code in (("identity", 0), ("nonzero", 1)):
        prep = api(node + "/operations", dict(type="command", name=name, parameters={}))
        task = "/api/v1/operations/" + prep["task_id"]
        api(task + "/confirm", dict(challenge=prep["challenge"]))
        wait(lambda: api(task)["finished_at"], "命令没有完成")
        detail = api(task)
        assert detail["exit_code"] == code and detail["state"] == (
            "failed" if code else "succeeded"
        ), detail
        if not code:
            assert detail["output"].strip() == user
    term = api(node + "/terminal", dict(password=password, cols=80, rows=24))
    path = "/api/v1/terminal/" + term["id"]
    api(path + "/resize", dict(cols=95, rows=30))
    api(
        path + "/input",
        dict(data=base64.b64encode(b"printf 'shell-%s-ok\\n' ssh; exit\n").decode()),
    )
    wait(
        lambda: b"shell-ssh-ok" in base64.b64decode(api(path + "?offset=0")["data"]),
        "SSH 终端输出缺失",
    )
    print("SSH 主机密钥、节点采样、资源面板、命令退出码和交互终端验证通过")
    check_local_panel(work, stack, user, ssh_port, port(), launch, wait)
    check_collector(work, stack, sshd, node, api, launch, wait, target)


if __name__ == "__main__":
    if os.geteuid() == 0:
        raise SystemExit("请以普通用户运行")
    root = Path(".local").resolve()
    root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=root) as directory, ExitStack() as stack:
        exercise(Path(directory), stack)
