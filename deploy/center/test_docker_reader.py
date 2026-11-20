"""Docker 检测、可选部署、重跑和只读清单验收。"""

import stat
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

import docker_probe
import docker_reader as reader
import host
import services


class DockerReaderTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def test_detect_standard_socket_without_cli(self):
        info = SimpleNamespace(st_uid=0, st_gid=312, st_mode=stat.S_IFSOCK | 0o660)
        with (
            patch.object(reader, "ENGINE", Mock(lstat=Mock(return_value=info))),
            patch.object(
                reader.grp, "getgrnam", return_value=SimpleNamespace(gr_gid=312)
            ),
        ):
            self.assertEqual(reader.detect()[0], "ready")
            for mode in (stat.S_IFREG | 0o660, stat.S_IFSOCK | 0o666):
                info.st_mode = mode
                self.assertEqual(reader.detect()[0], "skip")

    def test_absent_cli_only_and_stopped_engine(self):
        with (
            patch.object(reader, "ENGINE", self.root / "missing"),
            patch.object(Path, "glob", return_value=[]),
            patch.object(services, "property_value", return_value="not-found") as unit,
            patch.object(reader.shutil, "which", return_value=None) as which,
            patch.object(services, "active", return_value=False) as active,
        ):
            self.assertIn("未检测到", reader.detect()[1])
            which.return_value = "/usr/bin/docker"
            self.assertIn("检测到 Docker 程序", reader.detect()[1])
            unit.return_value = "loaded"
            self.assertEqual(reader.detect()[0], "pending")
            active.return_value = True
            self.assertEqual(reader.detect()[0], "skip")

    def test_skip_never_changes_configuration(self):
        with (
            patch.object(
                reader, "detect", return_value=("skip", "缺少 Engine")
            ) as detect,
            patch.object(reader, "ask", return_value="2") as ask,
            patch.object(reader, "check_install") as check,
            patch.object(host, "run") as run,
        ):
            reader.configure(self.root, self.root, self.root, selected=True)
            ask.assert_not_called()
            detect.return_value = ("ready", "检测到 Engine")
            reader.configure(self.root, self.root, self.root)
            ask.assert_called_once()
            check.assert_not_called()
            run.assert_not_called()

    def test_preserves_account_conflict_and_reuses_service_account(self):
        account = SimpleNamespace(pw_uid=1001, pw_gid=232, pw_shell="/bin/bash")
        with (
            patch.object(reader.pwd, "getpwnam", return_value=account),
            patch.object(
                reader.grp,
                "getgrnam",
                side_effect=lambda name: SimpleNamespace(
                    gr_gid=231 if name == "docker" else 232
                ),
            ),
            patch.object(host, "run") as run,
        ):
            with self.assertRaisesRegex(ValueError, "同名账号"):
                reader.reader_group()
            account.pw_uid = 231
            account.pw_shell = "/usr/sbin/nologin"
            self.assertEqual(reader.reader_group(), 232)
            run.assert_not_called()

    def test_existing_release_and_manual_configuration(self):
        base = self.root / "center"
        release = base / "releases/test"
        binary = release / "bin/asterion-docker-reader"
        host.put(binary, "test\n", 0o755)
        agent = self.root / "agent"
        agent.mkdir()
        (base / "current").symlink_to(release)
        (agent / "current").symlink_to(base / "current")
        host.put(self.root / "agent.env", "# Agent\n")
        units = self.root / "units"
        host.put(units / host.AGENT, (host.HERE.parent / host.AGENT).read_text(), 0o644)
        drop = units / (host.AGENT + ".d/30-docker-read.conf")
        host.put(drop, host.template("docker-read.conf"), 0o644)
        with (
            patch.object(host, "UNITS", units),
            patch.multiple(
                services, UNITS=units, RUNTIME=self.root / "runtime", DOCKER_CONF=drop
            ),
            patch.object(services, "active", return_value=True),
            patch.object(services, "property_value", return_value=""),
        ):
            reader.check_install(base, agent, self.root)
            drop.write_text("[Service]\nSupplementaryGroups=docker\n")
            with self.assertRaisesRegex(ValueError, "服务定义"):
                reader.check_install(base, agent, self.root)
            drop.unlink()
            (self.root / "agent.env").write_text(f"{reader.VARIABLE}=/custom\n")
            with self.assertRaisesRegex(ValueError, "其他 Docker"):
                reader.check_install(base, agent, self.root)
            binary.unlink()
            with self.assertRaisesRegex(ValueError, "缺少可执行"):
                reader.check_install(base, agent, self.root)

    def test_resume_after_files_written_before_agent_restart(self):
        units = self.root / "units"
        drop = units / "agent.service.d/30-docker-read.conf"
        env = self.root / "agent.env"
        host.put(env, "# 本机配置\n")
        access = {"user": 111, "group": 222, "extra_groups": [333]}
        with (
            patch.object(reader, "detect", return_value=("ready", "发现 Docker")),
            patch.object(reader, "check_install"),
            patch.object(reader, "reader_group", return_value=333),
            patch.object(
                reader, "agent_access", side_effect=[None, None, access, access]
            ),
            patch.object(services, "DOCKER_CONF", drop),
            patch.object(host, "UNITS", units),
            patch.object(host, "run"),
            patch.object(services, "ensure_running") as activate,
        ):
            activate.side_effect = [OSError("测试中断")]
            with self.assertRaises(OSError):
                reader.configure(self.root, self.root, self.root, selected=True)
            self.assertEqual(drop.read_text(), host.template("docker-read.conf"))
            activate.side_effect = None
            reader.configure(self.root, self.root, self.root, selected=True)
            activate.assert_called_with(host.AGENT, restart=True)
            activate.reset_mock()
            reader.configure(self.root, self.root, self.root, selected=True)
            activate.assert_called_once_with(reader.UNIT)
            self.assertEqual(
                env.read_text(), f"# 本机配置\n{reader.VARIABLE}={reader.SOCKET}\n"
            )

    def test_empty_engine_is_available_and_stopped_engine_is_pending(self):
        with patch.object(docker_probe, "inventory") as inventory:
            inventory.return_value = {
                "data": {"status": "available", "engine_id": "test", "containers": []}
            }
            docker_probe.verify("unused", "ready")
            inventory.return_value = {"data": {"status": "unreachable"}}
            docker_probe.verify("unused", "pending")
            with self.assertRaisesRegex(ValueError, "unreachable"):
                docker_probe.verify("unused", "ready")
            inventory.return_value = {"data": {"status": "permission_denied"}}
            with self.assertRaisesRegex(ValueError, "permission_denied"):
                docker_probe.verify("unused", "pending")
