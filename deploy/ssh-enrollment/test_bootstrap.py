import contextlib
import io
import json
import os
from pathlib import Path
import secrets
import socket
import tempfile
import threading
import unittest
from unittest.mock import patch

import bootstrap
import paramiko


class RecordingPeer(paramiko.ServerInterface):
    def __init__(self, request, key, fail_at=None):
        self.request = request
        self.key = key
        self.fail_at = fail_at
        self.auths = []
        self.commands = []
        self.threads = []
        self.channels = []
        self.disconnected = threading.Event()

    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_password(self, username, password):
        self.auths.append(username)
        if username == self.request["user"] and password == self.request["secret"]:
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def check_auth_publickey(self, username, key):
        self.auths.append(username)
        return paramiko.AUTH_SUCCESSFUL if key == self.key else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        command = command.decode()
        self.commands.append(command)
        def record():
            with channel:
                while channel.recv(65536):
                    pass
                if self.fail_at and self.fail_at in command:
                    channel.sendall(self.request["token"].encode())
                    channel.send_exit_status(71)
                else:
                    if "uname -sm" in command:
                        channel.sendall(b"Linux x86_64\n")
                    elif "curl --disable" in command:
                        channel.sendall(b'{"status":"ok"}')
                    elif "systemctl is-active" in command:
                        channel.sendall(b"active\n")
                    channel.send_exit_status(0)
        thread = threading.Thread(target=record)
        self.threads.append(thread)
        thread.start()
        return True


@contextlib.contextmanager
def endpoint(peer, server_key):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(5)
        def serve():
            sock, _ = listener.accept()
            with sock, contextlib.closing(paramiko.Transport(sock)) as transport:
                transport.add_server_key(server_key)
                try:
                    transport.start_server(server=peer)
                except (EOFError, ConnectionResetError):
                    # Fingerprint inspection can disconnect as soon as key exchange completes.
                    peer.disconnected.set()
                    return
                while transport.is_active():
                    channel = transport.accept(.1)
                    if channel is not None:
                        peer.channels.append(channel)
                for thread in peer.threads:
                    thread.join(2)
                peer.disconnected.set()
        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        try:
            yield listener.getsockname()[1]
        finally:
            thread.join(6)
            if thread.is_alive():
                raise AssertionError("SSH test peer did not disconnect")


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        sandbox = tempfile.TemporaryDirectory()
        self.addCleanup(sandbox.cleanup)
        binary = Path(sandbox.name)/"agent"
        binary.write_bytes(b"\x7fELF\x02\x01" + bytes(12) + b"\x3e\x00" + secrets.token_bytes(8192))
        config = patch.dict(os.environ, {
            "ASTERION_SSH_DEPLOY_CENTER_URL":"https://center.example.invalid",
            "ASTERION_SSH_DEPLOY_AGENT":str(binary),
            "ASTERION_SSH_DEPLOY_NETWORKS":"127.0.0.1/32",
        }, clear=True)
        config.start()
        self.addCleanup(config.stop)
        self.server_key = paramiko.RSAKey.generate(2048)
        self.user_key = paramiko.RSAKey.generate(2048)
        self.req = {"host":"127.0.0.1","user":"fixture","secret":secrets.token_hex(12),
                    "token":secrets.token_hex(32),"method":"password",
                    "unit":"[Service]\nDynamicUser=yes\n","mode":"inspect"}

    def exchange(self, fail_at=None):
        peer = RecordingPeer(self.req, self.user_key, fail_at)
        output = io.StringIO()
        with endpoint(peer, self.server_key) as port, contextlib.redirect_stdout(output):
            self.req["port"] = port
            bootstrap.run(self.req)
        self.assertTrue(peer.disconnected.is_set())
        return peer, [json.loads(line) for line in output.getvalue().splitlines()]

    def prepare(self):
        peer, events = self.exchange()
        self.assertEqual(peer.auths, [])
        self.assertEqual(peer.commands, [])
        self.req.update({name:events[0][name] for name in ("address","fingerprint","manifest")}, mode="deploy")
        return events[0]["commands"]

    def test_password_deployment_runs_only_reviewed_commands_and_disconnects(self):
        plan = self.prepare()
        peer, logs = self.exchange()
        self.assertEqual(plan, peer.commands)
        self.assertEqual(logs[-1]["line"], "SSH 会话已关闭")
        self.assertIn({"line": "Agent 传输：100%"}, logs)
        self.assertNotIn(self.req["secret"], str(logs))
        self.assertNotIn(self.req["token"], str(logs))
        self.assertTrue(all("systemctl enable" not in cmd or "asterion-agent.service" in cmd for cmd in plan))

    def test_encrypted_user_key_stays_in_memory(self):
        self.prepare()
        private = io.StringIO()
        password = secrets.token_hex(10)
        self.user_key.write_private_key(private, password=password)
        self.req.update(method="key", secret=private.getvalue(), passphrase=password)
        _, logs = self.exchange()
        self.assertNotIn("PRIVATE KEY", str(logs))
        self.assertNotIn(password, str(logs))

    def test_changed_key_is_rejected_before_authentication(self):
        self.prepare()
        self.server_key = paramiko.RSAKey.generate(2048)
        peer = RecordingPeer(self.req, self.user_key)
        with endpoint(peer, self.server_key) as port:
            self.req["port"] = port
            with self.assertRaisesRegex(bootstrap.Failure, "指纹已改变"):
                bootstrap.run(self.req)
        self.assertFalse(peer.auths or peer.commands)

    def test_failure_stops_before_install_and_never_echoes_remote_secrets(self):
        self.prepare()
        peer = RecordingPeer(self.req, self.user_key, "LoadState")
        output = io.StringIO()
        with endpoint(peer, self.server_key) as port, contextlib.redirect_stdout(output):
            self.req["port"] = port
            with self.assertRaisesRegex(bootstrap.Failure, "首次安装检查"):
                bootstrap.run(self.req)
        self.assertEqual(len(peer.commands), 2)
        self.assertNotIn(self.req["token"], output.getvalue())
        self.assertIn("71", output.getvalue())

    def test_network_policy_and_center_origin_fail_closed(self):
        with self.assertRaises(bootstrap.Failure):
            bootstrap.destination("127.0.0.1", 22, "192.0.2.0/24")
        with self.assertRaises(bootstrap.Failure):
            bootstrap.destination("127.0.0.1", 22, "127.0.0.1/32", "127.0.0.2")
        for bad in ["http://host", "https://user@host", "https://host/extra", "https://host?key=value"]:
            with self.assertRaises(bootstrap.Failure):
                bootstrap.origin(bad)

    def test_changed_artifact_rejected_without_sending_a_password(self):
        self.prepare()
        with Path(os.environ["ASTERION_SSH_DEPLOY_AGENT"]).open("ab") as binary:
            binary.write(b"changed")
        peer = RecordingPeer(self.req, self.user_key)
        with endpoint(peer, self.server_key) as port:
            self.req["port"] = port
            with self.assertRaisesRegex(bootstrap.Failure, "发布产物"):
                bootstrap.run(self.req)
        self.assertFalse(peer.auths or peer.commands)

    def test_private_ca_source_rejected_before_ssh(self):
        path = Path(os.environ["ASTERION_SSH_DEPLOY_AGENT"]).parent / "ca.pem"
        path.write_bytes(b"-----BEGIN CERTIFICATE-----\n-----BEGIN PRIVATE KEY-----")
        with patch.dict(os.environ, {"ASTERION_SSH_DEPLOY_CA_FILE":str(path)}):
            with self.assertRaisesRegex(bootstrap.Failure, "公开证书"):
                bootstrap.run(self.req)
