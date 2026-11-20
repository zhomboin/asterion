import fcntl
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "managed"))
import actions
import dispatch


class Requests(unittest.TestCase):
    def send(self, kind, args, policy=None):
        return dispatch.run(dict(kind=kind, args=args, policy=policy or {}))

    def test_sample_and_process(self):
        sample = self.send("sample", {})["value"]["sample"]
        self.assertGreater(sample["memory"]["total_bytes"], 0)
        row = self.send("process", {"pid": os.getpid()})["value"]
        self.assertGreater(row["start_ticks"], 0)

    def test_operations_are_scoped(self):
        for action in (
            dict(type="systemd", unit="missing.service", verb="restart"),
            dict(type="command", name="missing", parameters={}),
            dict(type="process", pid=os.getpid(), start_ticks=1, signal="kill"),
        ):
            self.assertEqual(
                self.send("operation_execute", {"action": action})["code"],
                "operation_denied",
            )
        self.assertEqual(
            self.send("managed_query", {"group": "configs", "key": "missing"})["code"],
            "operation_denied",
        )

    def test_predefined_parameters_and_output(self):
        policy = {
            "commands": [
                dict(
                    name="sample",
                    program="/usr/bin/printf",
                    args=["%s", {"parameter": "v"}],
                    parameters={"v": {"type": "integer", "min": 0, "max": 9}},
                )
            ]
        }
        action = dict(type="command", name="sample", parameters={"v": "3"})
        self.assertEqual(actions.execute(policy, {"action": action})["output"], "3")
        action["parameters"]["v"] = "invalid"
        self.assertEqual(
            self.send("operation_execute", {"action": action}, policy)["code"],
            "invalid_argument",
        )
        self.assertNotIn("private", actions.redact("password=private"))

    def test_identity_checks(self):
        self.assertEqual(
            self.send("docker_logs", {"id": "invalid", "lines": 10})["code"],
            "operation_denied",
        )
        self.assertEqual(
            self.send("account_keys", {"username": "root"})["code"], "operation_denied"
        )

    def test_command_paths(self):
        shell = Path("/bin/sh")
        self.assertEqual(actions.command_path(str(shell)), shell.resolve(strict=True))
        with tempfile.TemporaryDirectory(dir="/tmp") as tmp:
            alias = Path(tmp) / "tool"
            alias.symlink_to(shell)
            for path in (str(alias), "bin/sh", "/etc/passwd"):
                with self.subTest(path=path), self.assertRaises(actions.Refused):
                    actions.command_path(path)

    def test_managed_change_refuses_concurrent_mutation(self):
        with tempfile.TemporaryDirectory() as tmp:
            fd = os.open(tmp, os.O_RDONLY | os.O_DIRECTORY)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with (
                    patch.object(actions, "state_dir", return_value=Path(tmp)),
                    patch.object(actions.host, "install") as install,
                ):
                    args = {
                        "action": {
                            "type": "managed",
                            "change": {"key": "tool", "verb": "install"},
                        }
                    }
                    policy = {"managed": {"packages": {"tool": {"package": "tool"}}}}
                    self.assertEqual(
                        self.send("operation_execute", args, policy)["code"], "busy"
                    )
                    install.assert_not_called()
            finally:
                os.close(fd)

    def test_disconnect_finishes_managed_change_and_stops_command(self):
        script = """
import sys, time
from pathlib import Path
import dispatch
def mark(request):
    time.sleep(0.15)
    Path(sys.argv[2]).touch()
    return {}
dispatch.run = mark
dispatch.main({"kind": "operation_execute", "args": {"action": {"type": sys.argv[1]}}, "seconds": 3})
"""
        with tempfile.TemporaryDirectory() as tmp:
            for kind in ("managed", "command"):
                with self.subTest(kind=kind):
                    marker = Path(tmp) / kind
                    subprocess.run(
                        [sys.executable, "-B", "-c", script, kind, marker],
                        cwd=Path(__file__).parent,
                        env={
                            **os.environ,
                            "PYTHONPATH": str(Path(actions.host.__file__).parent),
                        },
                        stdin=subprocess.DEVNULL,
                        capture_output=True,
                        check=True,
                        timeout=5,
                    )
                    self.assertEqual(marker.exists(), kind == "managed")
