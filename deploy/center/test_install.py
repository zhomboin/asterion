"""中心首次安装的隔离回归测试。"""

import contextlib
import hashlib
import io
import json
import secrets
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, call, patch

import host
import inputs
import install

sys.path.insert(0, str(host.HERE.parent))
import self_monitor


class InstallTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.c = {
            "port": "18080",
            "domain": "panel.example.test",
            "node": "",
            "agent": "",
            "admin_ip": "192.0.2.1",
            "ops": "operator",
            "revision": "abc",
            "sha256": "a" * 64,
        }

    def archive(self, extra=None):
        archive = self.root / "asterion-abc.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for name in (
                "bin/asterion-server",
                "bin/asterion-agent",
                "web/index.html",
                "deploy/asterion-server.service",
                "deploy/asterion-agent.service",
                "deploy/self_monitor.py",
            ):
                item = tarfile.TarInfo(name)
                item.size = 1
                tar.addfile(item, io.BytesIO(b"x"))
            if extra:
                tar.addfile(extra)
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        Path(str(archive) + ".sha256").write_text(f"{checksum}  {archive.name}\n")
        return archive, checksum

    def test_verified_archive(self):
        archive, digest = self.archive()
        self.assertEqual(inputs.release_info(archive), ("abc", digest))
        stage = self.root / "release"
        inputs.unpack(archive, digest, stage)
        self.assertEqual((stage / "bin/asterion-agent").stat().st_mode & 0o777, 0o755)
        self.assertEqual((stage / "web/index.html").read_bytes(), b"x")

    def test_checksum_must_bind_to_the_archive(self):
        archive, _ = self.archive()
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            inputs.unpack(archive, "0" * 64, self.root / "release")
        Path(str(archive) + ".sha256").write_text("a" * 64 + "  other.tar.gz\n")
        with self.assertRaises(ValueError):
            inputs.release_info(archive)

    def test_rejects_archive_links_and_traversal(self):
        for name, kind in (
            ("web/../../escaped", tarfile.REGTYPE),
            ("web/link", tarfile.SYMTYPE),
        ):
            item = tarfile.TarInfo(name)
            item.type = kind
            item.linkname = "/etc"
            archive, digest = self.archive(item)
            stage = self.root / ("release-" + kind.hex())
            with self.assertRaises(ValueError):
                inputs.unpack(archive, digest, stage)
            self.assertFalse((self.root / "escaped").exists())

    def test_input_and_network_scope(self):
        for name in ("bad\nname.test", "site.test;include", ".", "*.example.test"):
            self.assertFalse(inputs.domain_ok(name))
        self.assertFalse(inputs.ipv4("0.0.0.0/0"))
        self.assertNotIn("SSH_DEPLOY", host.server_config(self.c))
        self.c["node"] = "192.0.2.2"
        self.assertIn("NETWORKS=192.0.2.2/32\n", host.server_config(self.c))

    def test_different_file_or_dangling_link_is_preserved(self):
        path = self.root / "config"
        host.put(path, "original")
        host.put(path, "original")
        with self.assertRaises(ValueError):
            host.put(path, "replacement")
        self.assertEqual(path.read_text(), "original")
        dangling = self.root / "current"
        dangling.symlink_to(self.root / "missing")
        with self.assertRaises(ValueError):
            install.fresh([dangling])

    def test_restart_uses_same_release_and_preserves_state(self):
        archive, digest = self.archive()
        self.c["sha256"] = digest
        stage = self.root / "stage"
        inputs.unpack(archive, digest, stage)
        with patch.multiple(
            install,
            BASE=self.root / "center",
            AGENT_BASE=self.root / "agent",
            UNITS=self.root / "units",
            AGENT_CONFIG=self.root / "agent-conf",
        ):
            install.install_release(self.c, stage)
            install.install_release(self.c, stage)
            self.assertEqual(
                (install.AGENT_BASE / "current").resolve(),
                (install.BASE / "releases/abc"),
            )
            (install.AGENT_CONFIG / "agent.env").write_text("管理员设置\n")
            with self.assertRaises(ValueError):
                install.install_release(self.c, stage)
            self.assertEqual(
                (install.AGENT_CONFIG / "agent.env").read_text(), "管理员设置\n"
            )

    def test_nginx_conflict_and_failure(self):
        nginx = "include /etc/nginx/conf.d/*.conf;\nserver_name\n 'panel.example.test';"
        with (
            patch.object(host, "run", return_value=Mock(stdout=nginx)),
            self.assertRaises(ValueError),
        ):
            host.check_site(self.c)
        site = self.root / "site.conf"
        with (
            patch.object(host, "site_path", return_value=site),
            patch.object(host, "check_site"),
            patch.object(
                host, "run", side_effect=subprocess.CalledProcessError(1, "nginx")
            ),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            host.enable_site(self.c)
        self.assertFalse(site.exists())

    def test_existing_certificate_verifies_renewal_without_prompt(self):
        renewal = self.root / "renewal"
        renewal.mkdir()
        (renewal / (self.c["domain"] + ".conf")).touch()
        with (
            patch.object(host, "LE", self.root),
            patch.object(host, "ask") as ask,
            patch.object(host, "run") as run,
        ):
            host.public_tls(self.c)
            ask.assert_not_called()
            run.assert_any_call(
                "certbot",
                "renew",
                "--cert-name",
                self.c["domain"],
                "--dry-run",
                "--run-deploy-hooks",
            )
        hook = self.root / "renewal-hooks/deploy" / ("asterion-" + self.c["domain"])
        self.assertIn("RENEWED_LINEAGE", hook.read_text())
        self.assertEqual(hook.stat().st_mode & 0o777, 0o755)

    def test_service_bootstrap_and_failure_cleanup(self):
        paths = {
            "CONFIG": self.root,
            "FIRST_ADMIN": self.root / "password",
            "ADMIN_CONF": self.root / "server.conf",
            "ENROLL_CONF": self.root / "agent.conf",
            "ENROLL": self.root / "enrollment",
            "IDENTITY": self.root / "identity",
        }
        paths["ENROLL"].mkdir()
        with (
            patch.multiple(install, **paths),
            patch.object(install, "run") as system,
            patch.object(install, "health"),
            patch.object(install, "monitor") as monitor,
        ):

            def enroll(command, *args):
                if command == "register":
                    self.assertEqual(install.FIRST_ADMIN.stat().st_mode & 0o777, 0o600)
                    (install.ENROLL / "token").touch()
                else:
                    self.assertTrue(
                        install.ENROLL_CONF.exists() or not install.FIRST_ADMIN.exists()
                    )

            monitor.side_effect = enroll
            install.enable_services(self.c, secrets.token_hex(12))
            self.assertIn(
                call("systemctl", "restart", install.SERVER, install.AGENT),
                system.call_args_list,
            )
            monitor.side_effect = RuntimeError("连接失败")
            with self.assertRaises(RuntimeError):
                install.enable_services(self.c, secrets.token_hex(12))
            self.assertIn(
                call("systemctl", "stop", install.AGENT, install.SERVER),
                system.call_args_list,
            )
            for path in (
                install.FIRST_ADMIN,
                install.ADMIN_CONF,
                install.ENROLL_CONF,
                install.ENROLL / "token",
            ):
                self.assertFalse(path.exists())

    def test_finished_install_only_checks_health(self):
        archive, _ = self.archive()
        state = self.root / "install.json"
        state.write_text(json.dumps(self.c))
        done = self.root / "complete"
        done.touch()
        with (
            patch.multiple(install, STATE=state, DONE=done),
            patch.object(install, "health") as health,
            patch.object(install, "run") as system,
        ):
            install.provision(archive)
            health.assert_called_once_with(self.c, public=True)
            system.assert_not_called()
            done.unlink()
            with self.assertRaisesRegex(ValueError, "相同"):
                install.provision(archive)
            system.assert_not_called()

    def test_reissue_only_for_recorded_pending_node(self):
        password = self.root / "password"
        password.write_text(secrets.token_hex(12))
        receipt = self.root / "receipt"
        node_id = secrets.token_hex(16)
        receipt.write_text(node_id)
        boot = self.root / "boot"
        boot.mkdir()
        expired = secrets.token_hex(32)
        (boot / "token").write_text(expired)
        node = {"id": node_id, "name": self_monitor.NODE_NAME, "status": "pending"}
        seen = []

        def request(req, timeout):
            path = req.full_url.split("/api/v1/")[1].split("?")[0]
            seen.append(path)
            if path == "nodes":
                body = {"has_more": False, "nodes": [node]}
            elif path.endswith("enrollment-token"):
                body = {"node_id": node_id, "token": secrets.token_hex(32)}
            else:
                body = {}
            stream = io.BytesIO(json.dumps(body).encode())
            stream.status = 200
            return contextlib.closing(stream)

        with (
            patch.multiple(self_monitor, BOOT_DIR=boot, PASSWORD=password),
            patch.object(self_monitor.ssl, "create_default_context"),
            patch.object(
                self_monitor.urllib.request,
                "build_opener",
                return_value=Mock(open=request),
            ),
            patch.object(self_monitor.os, "umask"),
        ):
            self_monitor.register(18080, "operator-panel", receipt)
            self.assertNotEqual((boot / "token").read_text().strip(), expired)
            self.assertIn(f"nodes/{node_id}/enrollment-token", seen)
            seen.clear()
            node["id"] = secrets.token_hex(16)
            with self.assertRaises(SystemExit):
                self_monitor.register(18080, "operator-panel", receipt)
            self.assertFalse(any("enrollment-token" in path for path in seen))


if __name__ == "__main__":
    unittest.main()
