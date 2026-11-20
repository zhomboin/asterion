import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "managed"))
import actions
from common import Refused, digest
import nginx_config
import text_files


class FileEditor(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).resolve().parents[2] / ".local"
        root.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(dir=root)
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.state = self.work / "backups"
        self.state.mkdir()
        self.path = self.work / "a file.conf"
        self.path.write_bytes(b"before\r\n")
        self.path.chmod(0o640)

    def change(self, entry, text, verb="save_text"):
        return dict(
            key=entry["path"],
            verb=verb,
            revision=entry["revision"],
            digest=digest(text.encode()),
        )

    def test_symlink_round_trip_and_stale_revision(self):
        link = self.work / "enabled"
        link.symlink_to(self.path)
        entry = text_files.view(str(link), False)
        self.assertEqual(entry["content"], "before\r\n")
        self.assertEqual(entry["resolved"], str(self.path))
        content = "中文\r\n"
        change = self.change(entry, content)
        self.assertEqual(text_files.save(change, content, self.state)["exit_code"], 0)
        self.assertTrue(link.is_symlink())
        self.assertEqual(self.path.read_bytes(), content.encode())
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o640)
        self.assertEqual(next(self.state.iterdir()).read_bytes(), b"before\r\n")
        with self.assertRaises(Refused) as caught:
            text_files.save(change, content, self.state)
        self.assertEqual(caught.exception.code, "target_changed")

    def test_boundaries_and_restricted_profiles(self):
        for group in ("files", "nginx"):
            with self.assertRaises(Refused):
                actions.managed_query({}, dict(group=group, key=str(self.path)))
        cmd = self.change(text_files.view(str(self.path), False), "new")
        with self.assertRaises(Refused):
            actions.managed({}, cmd, "new")
        for data in (b"\x00", b"\xff", b"x" * (65536 + 1)):
            self.path.write_bytes(data)
            with self.assertRaises(Refused):
                text_files.view(str(self.path), False)
        self.path.write_bytes(b"x" * 65536)
        entry = text_files.view(str(self.path), False)
        with self.assertRaises(Refused):
            text_files.save(self.change(entry, "first"), "second", self.state)
        os.link(self.path, self.work / "hard")
        with self.assertRaises(Refused):
            text_files.save(self.change(entry, ""), "", self.state)
        fifo = self.work / "pipe"
        os.mkfifo(fifo)
        for path in (fifo, self.work, "relative"):
            with self.assertRaises(Refused):
                text_files.view(str(path), False)

    def test_rollback_does_not_overwrite_external_edit(self):
        with patch.object(
            nginx_config, "selected", return_value=(1, "identity", "nginx", [])
        ):
            entry = text_files.view(str(self.path), True)

            def intervene(*_):
                self.path.write_text("external")
                return "校验失败"

            with patch.object(nginx_config, "reload_checked", side_effect=intervene):
                result = text_files.save(
                    self.change(entry, "new", "save_nginx"), "new", self.state
                )
            self.assertEqual(result["exit_code"], 1)
            self.assertEqual(self.path.read_text(), "external")

    @unittest.skipUnless(os.environ.get("TEST_NGINX"), "需要指定隔离测试的 Nginx 程序")
    def test_real_nginx_discovery_validation_and_reload(self):
        binary = str(Path(os.environ["TEST_NGINX"]).resolve(strict=True))
        config = self.work / "main.conf"
        config.write_text(
            "pid nginx.pid;\nerror_log error.log;\nevents { include enabled/*.conf; }\n"
        )
        enabled = self.work / "enabled"
        enabled.mkdir()
        site = enabled / "site.conf"
        site.symlink_to(self.path)
        self.path.write_text("worker_connections 100;\n# configuration file /fake:\n")
        (self.work / "unused.conf").write_text("# 未引用")
        process = subprocess.Popen(
            [
                binary,
                "-p",
                str(self.work),
                "-c",
                str(config),
                "-e",
                str(self.work / "error.log"),
                "-g",
                "daemon off;",
            ]
        )
        try:
            proc = Path("/proc") / str(process.pid)
            children = proc / "task" / str(process.pid) / "children"

            def workers():
                return set(children.read_text().split())

            def wait(check):
                for _ in range(200):
                    if process.poll() is not None:
                        self.fail("Nginx 测试实例意外退出")
                    if check():
                        return
                    time.sleep(0.025)
                self.fail("等待 Nginx 工作进程超时")

            wait(workers)
            original_workers = workers()
            discover = nginx_config.master
            for candidates, code in (
                ([], "nginx_missing"),
                ([proc, proc], "nginx_multiple"),
            ):
                with self.assertRaises(Refused) as raised:
                    discover(candidates)
                self.assertEqual(raised.exception.code, code)
            with patch.object(
                nginx_config, "master", side_effect=lambda: discover([proc])
            ):
                self.assertEqual(
                    nginx_config.paths(nginx_config.master()), [str(config), str(site)]
                )
                entry = text_files.view(str(site), True)
                invalid = "invalid_directive on;"
                fail = text_files.save(
                    self.change(entry, invalid, "save_nginx"), invalid, self.state
                )
                self.assertEqual(fail["exit_code"], 1)
                self.assertIn(str(site) + ":1", fail["output"])
                self.assertEqual(self.path.read_text(), entry["content"])
                self.assertEqual(workers(), original_workers)
                valid = "worker_connections 256;\n"
                ok = text_files.save(
                    self.change(entry, valid, "save_nginx"), valid, self.state
                )
                self.assertEqual(ok["exit_code"], 0, ok)
                wait(lambda: workers() - original_workers)
                self.assertTrue(site.is_symlink())
                self.assertEqual(site.read_text(), valid)
        finally:
            process.terminate()
            process.wait(timeout=5)
