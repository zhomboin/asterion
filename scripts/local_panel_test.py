"""验证仅在本机启动的面板、SSH 别名与无 Python 节点。"""

import base64
import http.cookiejar
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.error
import urllib.request
from ssh_auth_test import check_auth


def check_local_panel(work, stack, user, ssh_port, port, launch, wait):
    commands = work / "base-tools"
    commands.mkdir()
    for name in ("id", "uname", "awk", "sleep", "df", "date"):
        (commands / name).symlink_to(shutil.which(name))
    subprocess.run(
        ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", work / "minimal"],
        check=True,
    )
    wrapper = work / "limited-shell"
    wrapper.write_text(
        f'#!/bin/sh\nexport PATH="{commands}"\nexec /bin/sh -c "$SSH_ORIGINAL_COMMAND"\n'
    )
    wrapper.chmod(0o700)
    with (work / "key.pub").open("a") as authorized:
        authorized.write(f'command="{wrapper}" ' + (work / "minimal.pub").read_text())
    config = work / "client.conf"
    (work / "revoked_hosts").write_text(
        "@revoked " + (work / "known_hosts").read_text()
    )
    config.write_text(f"""Host minimal
  IdentityFile {work}/minimal
Host bad-key
  UserKnownHostsFile {work}/wrong_hosts
Host first-use
  UserKnownHostsFile {work}/new_hosts
Host cannot-save
  UserKnownHostsFile /dev/null
Host revoked
  UserKnownHostsFile {work}/revoked_hosts
Host *
  HostName 127.0.0.1
  Port {ssh_port}
  User {user}
  IdentitiesOnly yes
  UserKnownHostsFile {work}/known_hosts
Host explicit-key
  IdentityFile none
Host !minimal !explicit-key *
  IdentityFile {work}/key
""")
    config.chmod(0o600)
    base = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({}),
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
    )

    def request(path, data=None, method=None, **headers):
        req = urllib.request.Request(
            base + path,
            method=method,
            data=json.dumps(data).encode() if data is not None else None,
            headers={
                "Content-Type": "application/json",
                "X-Asterion-CSRF": "1",
                **headers,
            },
        )
        with opener.open(req, timeout=12) as response:
            if path == "/api/v1/auth/login":
                assert not response.headers.get("Set-Cookie")
                opener.addheaders = [
                    (
                        "X-Asterion-Local-Session",
                        response.headers["X-Asterion-Local-Session"],
                    )
                ]
            content = response.read()
            return json.loads(content) if content else None

    env = {
        **os.environ,
        "ASTERION_LOCAL": "1",
        "ASTERION_LISTEN_ADDR": f"127.0.0.1:{port}",
        "ASTERION_ADMIN_USERNAME": "local",
        "ASTERION_ADMIN_PASSWORD_FILE": str(work / "password"),
        "ASTERION_DATABASE_URL": "sqlite://" + str(work / "local.db"),
        "ASTERION_LOCAL_SSH_CONFIG": str(config),
        "ASTERION_SSH_CONFIG_FILE": str(work / "direct-profiles.json"),
    }
    binary = os.environ.get("TEST_CENTER", "target/debug/asterion-server")
    unmanaged = env.copy()
    del unmanaged["ASTERION_LOCAL"]
    denied = subprocess.run([binary], env=unmanaged, capture_output=True, timeout=10)
    assert denied.returncode and b"start.sh" in denied.stderr
    denied = subprocess.run(
        [binary],
        env={**env, "ASTERION_LISTEN_ADDR": "0.0.0.0:0"},
        capture_output=True,
        timeout=10,
    )
    assert denied.returncode and "回环" in denied.stderr.decode()
    process = launch(stack, [binary], work / "local.log", env)

    def started():
        if process.poll() is not None:
            raise RuntimeError((work / "local.log").read_text())
        try:
            return request("/healthz")["status"] == "ok"
        except urllib.error.URLError:
            return False

    wait(started, "本机面板未启动")
    for headers in ({"Host": "other.invalid"}, {"Origin": "http://other.invalid"}):
        try:
            request("/healthz", **headers)
        except urllib.error.HTTPError as error:
            assert error.code == 403
        else:
            raise AssertionError("本机面板错误接受了其他站点的请求")
    password = (work / "password").read_text()
    try:
        request("/api/v1/ssh/host-key", {"ssh_profile": "ssh://first-use"})
    except urllib.error.HTTPError as error:
        assert error.code == 401
    else:
        raise AssertionError("指纹核对需要面板登录")
    request("/api/v1/auth/login", dict(username="local", password=password))
    assert request("/api/v1/settings")["local_ssh"]
    assert request("/api/v1/ssh/profiles") == []
    check_host_keys(work, request)

    def node(address):
        return (
            "/api/v1/nodes/"
            + request(
                "/api/v1/nodes",
                dict(name=address, kind="linux", ssh_profile="ssh://" + address),
            )["id"]
        )

    direct = node(f"{user}@127.0.0.1:{ssh_port}")
    minimal = node("minimal")
    wrong = node("bad-key")
    wait(lambda: request(direct)["status"] == "online", "本机直连失败")
    policy = {"managed": {"packages": {"shell": {"package": "bash"}}}}
    (work / "direct-profiles.json").write_text(
        json.dumps({f"ssh://{user}@127.0.0.1:{ssh_port}": {"policy": policy}})
    )

    def policy_loaded():
        try:
            return "packages" in request(direct + "/operation-catalog").get(
                "managed", []
            )
        except urllib.error.HTTPError as error:
            if error.code != 503:
                raise
            return False

    wait(policy_loaded, "直连节点未加载专用面板策略")
    assert request(direct + "/managed/packages?key=shell")["installed"]
    wait(
        lambda: request(wrong)["status"] == "offline", "本机连接必须拒绝错误的主机密钥"
    )
    wait(lambda: request(minimal)["status"] == "online", "无 Python 节点未上线")
    sample = request(minimal + "/metrics")["samples"][-1]["sample"]
    assert (
        sample["memory"]["total_bytes"] > 0
        and sample["filesystems"][0]["mountpoint"] == "/"
    )
    assert request(minimal + "/terminal/capability")["enabled"]
    tid = request(minimal + "/terminal", dict(password=password, cols=80, rows=24))[
        "id"
    ]
    term = "/api/v1/terminal/" + tid
    request(
        term + "/input",
        dict(
            data=base64.b64encode(b"printf 'local-%s-ok\\n' terminal; exit\n").decode()
        ),
    )
    wait(
        lambda: (
            b"local-terminal-ok"
            in base64.b64decode(request(term + "?offset=0")["data"])
        ),
        "本机 SSH 终端缺少执行结果",
    )
    print("本机 HTTP 登录、来源校验、SSH 地址与别名、无 Python 采样和交互终端验证通过")
    check_auth(work, user, ssh_port, request, wait, sample)
    print("指定私钥、SSH 密码认证、拒绝后暂停及更换密码验证通过")


def check_host_keys(work, request):
    endpoint = "/api/v1/ssh/host-key"
    payload = {"ssh_profile": "ssh://first-use"}
    key = request(endpoint, payload)
    fingerprint = subprocess.check_output(
        ["ssh-keygen", "-lf", work / "host.pub", "-E", "sha256"], text=True
    ).split()[1]
    assert key == {
        "status": "unknown",
        "algorithm": "ED25519",
        "fingerprint": fingerprint,
    }, key
    hosts = work / "new_hosts"
    assert not hosts.exists() or not hosts.read_bytes()

    def rejected(data):
        try:
            request(endpoint, data)
        except urllib.error.HTTPError as error:
            assert error.code == 409, error
        else:
            raise AssertionError("不应接受该主机密钥")

    other = subprocess.check_output(
        ["ssh-keygen", "-lf", work / "key.pub"], text=True
    ).split()[1]
    rejected({**payload, "fingerprint": other})
    assert not hosts.exists() or not hosts.read_bytes()
    trusted = {"status": "trusted"}
    assert request(endpoint, {**payload, "fingerprint": fingerprint}) == trusted
    assert hosts.stat().st_size and request(endpoint, payload) == trusted
    rejected({**payload, "fingerprint": other})
    rejected({"ssh_profile": "ssh://bad-key"})
    rejected({"ssh_profile": "ssh://bad-key", "fingerprint": fingerprint})
    rejected({"ssh_profile": "ssh://revoked", "fingerprint": fingerprint})
    rejected({"ssh_profile": "ssh://cannot-save", "fingerprint": fingerprint})
    print("已验证首次指纹、取消、确认保存、已信任主机与密钥冲突")
