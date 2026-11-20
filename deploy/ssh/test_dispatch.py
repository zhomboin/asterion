import os
from pathlib import Path
import sys
import unittest

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
