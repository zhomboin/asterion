import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import bootstrap


class ReinstallTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.app = root / "app"
        self.original = self.app / "releases" / "initial"
        (self.original / "bin").mkdir(parents=True)
        (self.original / "bin/asterion-agent").write_bytes(b"original")
        (self.original / "bin/helper").write_bytes(b"helper")
        (self.app / "current").symlink_to(self.original)
        self.node = "ab" * 16
        self.identity = root / "identity.json"
        endpoint = "wss://center.example.invalid/api/v1/agent/connect"
        self.identity.write_text(json.dumps({"node_id": self.node, "endpoint": endpoint}))
        center = "https://center.example.invalid:443"
        self.binary = b"new agent"
        digest = hashlib.sha256(self.binary).hexdigest()
        with patch.multiple(bootstrap, APP=str(self.app), STATE=str(root), CONF=str(root)):
            plan = bootstrap.reinstall_steps(center, None, digest, self.node)
        self.install = dict((title, cmd) for title, cmd, _ in plan)["传输 Agent"]
        self.env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"])
        # The fixture acts as the owner of its temporary release directories.
        for name, body in {
            "stat": "echo 0",
            "systemctl": 'test "${FAIL_RESTART:-no}" != yes || test "$(cat "$FIXTURE_AGENT")" = original',
        }.items():
            path = root / name
            path.write_text("#!/bin/sh\n" + body + "\n")
            path.chmod(0o700)
        self.agent = self.app / "current/bin/asterion-agent"
        self.env["FIXTURE_AGENT"] = str(self.agent)

    def deploy(self, data=None):
        result = subprocess.run(["/bin/sh", "-c", self.install], env=self.env,
                                input=self.binary if data is None else data, capture_output=True)
        return result.returncode

    def test_switch_preserves_identity_helpers_and_previous_release(self):
        original_identity = self.identity.read_bytes()
        self.assertEqual(self.deploy(), 0)
        release = (self.app / "current").resolve()
        self.assertEqual(self.agent.read_bytes(), self.binary)
        self.assertEqual((release / "bin/helper").read_bytes(), b"helper")
        self.assertEqual(self.identity.read_bytes(), original_identity)
        self.assertEqual((self.original / "bin/asterion-agent").read_bytes(), b"original")
        self.assertEqual(self.deploy(), 0)
        self.assertEqual((self.app / "current").resolve(), release)

    def test_wrong_node_corrupt_transfer_and_failed_restart_keep_original(self):
        self.assertNotEqual(self.deploy(b"partial"), 0)
        self.env["FAIL_RESTART"] = "yes"
        self.assertNotEqual(self.deploy(), 0)
        identity = json.loads(self.identity.read_text())
        self.identity.write_text(json.dumps(dict(identity, node_id="00" * 16)))
        self.env.pop("FAIL_RESTART")
        self.assertNotEqual(self.deploy(), 0)
        self.identity.write_text(json.dumps(dict(identity, endpoint="wss://other.example.invalid/api/v1/agent/connect")))
        self.assertNotEqual(self.deploy(), 0)
        self.assertEqual(list(self.original.parent.iterdir()), [self.original])
        self.assertEqual(self.agent.read_bytes(), b"original")
