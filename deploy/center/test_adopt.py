"""手动安装各阶段与再次运行的隔离回归。"""

import hashlib
import json
import secrets
import shutil
import sqlite3
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, call, patch

import existing
import host
import install
import pki
import services


class AdoptionTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.config = self.root / "config"
        self.config.mkdir(mode=0o750)
        self.setup = self.root / "setup"
        self.setup.mkdir(mode=0o700)
        self.c = {
            "domain": "panel.example.test",
            "port": "18080",
            "ops": "test-ops",
            "node": "192.0.2.2",
            "agent": "192.0.2.9",
            "admin_ip": "192.0.2.9",
        }

    def test_manual_account_needs_neither_key_nor_adduser(self):
        account = SimpleNamespace(
            pw_name=self.c["ops"], pw_dir=str(self.root), pw_gecos=""
        )
        with (
            patch.object(host, "SUDOERS", self.config),
            patch.object(host, "login_account", return_value=account),
            patch.object(host, "authorized_keys", return_value=True),
            patch.object(host, "public_key") as key,
            patch.object(host, "run") as run,
        ):
            host.operator(self.c, self.root)
            rule = next(self.config.iterdir())
            previous = rule.stat()
            host.operator(self.c, self.root)
            self.assertEqual(rule.stat().st_ino, previous.st_ino)
            self.assertEqual(rule.stat().st_mode & 0o777, 0o440)
            key.assert_not_called()
            self.assertEqual(
                run.call_args_list, [call("visudo", "-cf", self.root / "sudoers")] * 2
            )

    def test_env_and_manual_acl_are_detected_and_preserved(self):
        env = self.config / "server.env"
        lines = host.server_config(self.c).splitlines()
        original = "# 保留注释\n" + "\n".join(reversed(lines[1:])) + "\n"
        host.put(env, original)
        existing.ensure_env(env, host.server_config(self.c), host.put)
        self.assertEqual(env.read_text(), original + lines[0] + "\n")
        with patch.object(host, "SITES", self.root):
            site = host.site_path(self.c)
            text = host.site_config(self.c).replace(
                "    deny all;", f"    allow {self.c['agent']};\n    deny all;"
            )
            host.put(site, text, 0o644)
            hints = existing.hints(self.config, self.root, self.root)
            for key in ("domain", "port", "node", "agent", "admin_ip"):
                self.assertEqual(hints[key], self.c[key])
            with patch.object(host, "run") as run:
                host.enable_site(self.c, lambda: True)
                run.assert_not_called()
            self.assertEqual(site.read_text(), text)
            site.write_text(text.replace("deny all;", "allow all;"))
            with self.assertRaises(ValueError):
                host.check_existing_site(self.c)

    def test_release_with_no_marker_partial_and_conflicting_files(self):
        stage = self.root / "stage"
        (stage / "bin").mkdir(parents=True)
        (stage / "bin/app").write_bytes(b"app")
        (stage / "web").mkdir()
        (stage / "web/index.html").write_text("page")
        release = self.root / "release"
        shutil.copytree(stage, release)
        exe = release / "bin/app"
        inode = exe.stat().st_ino
        page = release / "web/index.html"
        page.unlink()
        exe.write_text("different")
        with self.assertRaises(ValueError):
            existing.reconcile_release(stage, release, "digest", host.put)
        self.assertFalse(page.exists())
        exe.write_text("app")
        existing.reconcile_release(stage, release, "digest", host.put)
        self.assertEqual(exe.stat().st_ino, inode)
        self.assertEqual(page.read_text(), "page")
        self.assertEqual((release / ".sha256").read_text(), "digest\n")

    def test_manual_staged_pki_and_missing_leaf(self):
        phrase = secrets.token_hex(16) + "\n"
        with patch.object(pki, "ca_password", return_value=phrase):
            pki.ensure_tls(self.config, self.setup, self.root)
        original = {
            name: (self.config / name).read_bytes()
            for name in ("tls.key", "tls.crt", "deploy-ca.crt", "pki/ca.key")
        }
        for name, temporary in (
            ("tls.crt", "tls.crt"),
            ("deploy-ca.crt", "ca.crt"),
            ("tls.key", "tls.key"),
        ):
            (self.config / name).rename(self.setup / temporary)
        with patch.object(
            pki, "ca_password", side_effect=AssertionError("重复索取口令")
        ):
            pki.ensure_tls(self.config, self.setup, self.root)
        for name, content in original.items():
            self.assertEqual((self.config / name).read_bytes(), content)
        (self.config / "tls.crt").unlink()
        (self.setup / "tls.crt").unlink()
        with patch.object(pki, "ca_password", return_value=phrase) as ask:
            pki.ensure_tls(self.config, self.setup, self.root)
            ask.assert_called_once_with(False)
        self.assertEqual((self.config / "tls.key").read_bytes(), original["tls.key"])
        self.assertEqual(
            (self.config / "pki/ca.key").read_bytes(), original["pki/ca.key"]
        )

    def test_manual_credentials_from_database_and_healthy_services(self):
        db = self.root / "center.db"
        identity = self.root / "identity"
        token = secrets.token_hex(32)
        node = secrets.token_hex(16)
        with sqlite3.connect(db) as conn:
            conn.executescript("""
                CREATE TABLE _sqlx_migrations (version INTEGER);
                CREATE TABLE panel_admin (id INTEGER, username TEXT);
                CREATE TABLE nodes (id TEXT, name TEXT, last_seen_at INTEGER);
                CREATE TABLE node_enrollment_tokens (node_id TEXT, fingerprint BLOB, expires_at INTEGER, consumed_at INTEGER, revoked_at INTEGER);
                CREATE TABLE agent_credentials (node_id TEXT, fingerprint BLOB, revoked_at INTEGER);
            """)
            conn.execute("INSERT INTO panel_admin VALUES(1,'panel-test')")
            conn.execute("INSERT INTO nodes VALUES(?, '中心服务器', NULL)", (node,))
            conn.execute(
                "INSERT INTO node_enrollment_tokens VALUES(?,?,?,NULL,NULL)",
                (node, hashlib.sha256(token.encode()).digest(), time.time() + 60),
            )
        boot = self.root / "boot"
        boot.mkdir()
        host.put(boot / "token", token)
        with patch.multiple(
            services,
            CONFIG=self.config,
            DATABASE=db,
            IDENTITY=identity,
            ENROLL=boot,
            ADMIN_CONF=self.root / "admin.conf",
            ENROLL_CONF=self.root / "enroll.conf",
            FIRST_ADMIN=self.root / "password",
        ):
            self.assertEqual(services.enrollment_state(), "token")
            self.assertEqual((self.config / "self-node").read_text().strip(), node)
            with sqlite3.connect(db) as conn:
                conn.execute("UPDATE node_enrollment_tokens SET expires_at=0")
            self.assertEqual(services.enrollment_state(), "register")
            bearer = secrets.token_hex(32)
            host.put(
                identity,
                json.dumps(
                    {
                        "node_id": node,
                        "endpoint": "wss://127.0.0.1:18080/api/v1/agent/connect",
                        "bearer": bearer,
                    }
                ),
            )
            with sqlite3.connect(db) as conn:
                conn.execute(
                    "INSERT INTO agent_credentials VALUES(?,?,NULL)",
                    (node, hashlib.sha256(bearer.encode()).digest()),
                )
            self.assertEqual(existing.local_identity(identity, "18080"), node)
            with (
                patch.object(services, "ensure_running") as ensure,
                patch.object(services, "active", return_value=True),
                patch.object(services, "run") as commands,
                patch.object(services, "password") as password,
            ):
                monitor = Mock()
                services.activate(self.c, Mock(), monitor)
                self.assertEqual(
                    ensure.call_args_list,
                    [call(services.SERVER), call(services.AGENT, restart=False)],
                )
                monitor.assert_called_once_with("wait")
                self.assertFalse(
                    any(
                        "restart" in op.args or "stop" in op.args
                        for op in commands.call_args_list
                    )
                )
                password.assert_not_called()
            self.assertEqual(existing.admin_name(db), "panel-test")

    def test_native_loader_accepts_distribution_binary(self):
        stage = self.root / "bin"
        stage.mkdir()
        for name in ("asterion-server", "asterion-agent"):
            shutil.copyfile("/bin/true", stage / name)
            (stage / name).chmod(0o755)
        install.check_binaries(self.root)
        (stage / "asterion-agent").write_bytes(b"incompatible")
        with self.assertRaisesRegex(ValueError, "CPU"):
            install.check_binaries(self.root)
