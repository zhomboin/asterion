"""在回环地址验证 SSH 密码，无需创建或修改系统账号。"""

from contextlib import contextmanager
import hmac
import json
import secrets
import socket
import threading
import urllib.error

import paramiko


class PasswordServer(paramiko.ServerInterface):
    def __init__(self, user, password):
        self.user = user
        self.password = password
        self.attempts = 0

    def get_allowed_auths(self, username):
        return "password"

    def check_auth_password(self, username, password):
        self.attempts += 1
        if username == self.user and hmac.compare_digest(password, self.password):
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        if kind == "session":
            return paramiko.OPEN_SUCCEEDED
        return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        return True


@contextmanager
def password_server(work, user, password, sample):
    key = paramiko.RSAKey.generate(2048)
    auth = PasswordServer(user, password)
    stopped = threading.Event()
    errors = []
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]
        listener.settimeout(0.2)
        with (work / "known_hosts").open("a") as hosts:
            hosts.write(f"[127.0.0.1]:{port} {key.get_name()} {key.get_base64()}\n")

        def serve():
            try:
                while not stopped.is_set():
                    try:
                        client, _ = listener.accept()
                    except socket.timeout:
                        continue
                    with client, paramiko.Transport(client) as transport:
                        transport.add_server_key(key)
                        transport.start_server(server=auth)
                        channel = transport.accept(4)
                        if channel is not None:
                            with channel:
                                channel.settimeout(5)
                                with channel.makefile("rb") as stream:
                                    request = json.loads(stream.readline(520001))
                                assert request["kind"] == "sample"
                                reply = {
                                    "outcome": "success",
                                    "value": {"sample": sample},
                                }
                                channel.sendall(json.dumps(reply).encode() + b"\n")
                                channel.send_exit_status(0)
            except Exception as error:
                errors.append(error)

        thread = threading.Thread(target=serve)
        thread.start()
        try:
            yield port, auth
        finally:
            stopped.set()
            thread.join(8)
            assert not thread.is_alive()
            if errors:
                raise errors[0]


def check_auth(work, user, ssh_port, request, wait, sample):
    key = {"method": "key", "identity_file": str(work / "key")}
    direct = f"ssh://{user}@explicit-key:{ssh_port}"
    node = request(
        "/api/v1/nodes",
        {"name": "key", "kind": "linux", "ssh_profile": direct, "ssh_auth": key},
    )
    assert node["ssh_auth"] == key
    path = "/api/v1/nodes/" + node["id"]
    wait(lambda: request(path)["status"] == "online", "指定私钥未能采样")
    request(path, method="DELETE")

    password = secrets.token_urlsafe(32)
    with password_server(work, user, password, sample) as (port, server):
        profile = f"ssh://{user}@127.0.0.1:{port}"
        node = request(
            "/api/v1/nodes",
            {
                "name": "password",
                "kind": "linux",
                "ssh_profile": profile,
                "ssh_auth": {"method": "password", "password": secrets.token_hex(16)},
            },
        )
        path = "/api/v1/nodes/" + node["id"]
        assert node["ssh_auth"] == {"method": "password"}
        wait(lambda: "暂停" in (request(path)["ssh_error"] or ""), "密码被拒绝后未暂停")
        assert server.attempts == 1
        request(
            path,
            {"ssh_auth": {"method": "password", "password": password}},
            method="PATCH",
        )
        wait(lambda: request(path)["status"] == "online", "密码认证未能完成采样")
        assert request(path)["ssh_auth"] == {"method": "password"}
        request(path, {"ssh_auth": {"method": "password"}}, method="PATCH")
        try:
            request(
                path,
                {"ssh_profile": direct, "ssh_auth": {"method": "password"}},
                method="PATCH",
            )
        except urllib.error.HTTPError as error:
            assert error.code == 400
        else:
            raise AssertionError("不应将已保存密码用于新的 SSH 目标")
        assert request(path)["ssh_profile"] == profile
        request(path, method="DELETE")
