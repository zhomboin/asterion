"""节点操作向导的授权范围、配置保留和重复执行。"""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import docker_probe
import host
import managed_setup as setup
import managed_targets as targets
import services


class ManagedSetupTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def test_choices_skip_or_decline_without_mutations(self):
        with (
            patch("builtins.input", return_value=""),
            patch.object(setup.docker_reader, "installed_release") as release,
            patch.object(host, "run") as run,
        ):
            setup.configure(self.root, self.root, self.root)
            release.assert_not_called()
            run.assert_not_called()
        with (
            patch("builtins.input", side_effect=["1", "0"]),
            patch.object(setup.docker_reader, "installed_release"),
            patch.object(setup, "check_release"),
            patch.object(
                setup,
                "prepare",
                return_value=setup.Instance(
                    "configs", self.root, "unit", self.root, "", {}, {}
                ),
            ),
            patch.object(host, "run") as run,
            patch.object(host, "put") as put,
        ):
            setup.configure(self.root, self.root, self.root)
            run.assert_not_called()
            put.assert_not_called()

    def test_merge_only_selected_endpoints_and_preserve_dormant_policy(self):
        path, drop = self.root / "policy", self.root / "cred"
        original = {
            "systemd_units": ["demo.service"],
            "managed": {"containers": setup.socket_name("containers")},
        }
        host.put(path, json.dumps(original))
        with self.assertRaisesRegex(ValueError, "其他授权"):
            setup.executor_policy(path, ["configs"], drop, setup.read_policy(path))
        host.put(drop, host.template("operations.conf"))
        with (
            patch.object(setup, "live_credential", return_value=None),
            self.assertRaisesRegex(ValueError, "其他授权"),
        ):
            setup.executor_policy(path, ["configs"], drop, original)
        with patch.object(setup, "live_credential", return_value=path.read_bytes()):
            new = setup.executor_policy(path, ["configs"], drop, original)
        self.assertEqual(new["systemd_units"], original["systemd_units"])
        self.assertEqual(
            new["managed"],
            {**original["managed"], "configs": setup.socket_name("configs")},
        )
        setup.publish_json(path, new)
        inode = path.stat().st_ino
        setup.publish_json(path, new)
        self.assertEqual(inode, path.stat().st_ino)
        path.write_text('{"managed":{"configs":"/custom"}}')
        with self.assertRaisesRegex(ValueError, "其他操作端点"):
            setup.executor_policy(path, ["configs"], drop, setup.read_policy(path))
        path.write_text('{"managed":{},"managed":{}}')
        with self.assertRaisesRegex(ValueError, "重复键"):
            setup.read_policy(path)

    def test_manual_group_drop_in_accepts_only_known_memberships(self):
        directory = self.root / (host.AGENT + ".d")
        path = directory / "override.conf"
        group = "[Service]\nSupplementaryGroups=asterion-ops-configs\n"
        host.put(path, group, 0o644)
        with patch.object(services, "UNITS", self.root):
            self.assertEqual(services.agent_overrides()[path], group)
            path.write_text(group + "SupplementaryGroups=docker\n")
            self.assertNotIn(path, services.agent_overrides())

    def test_interactive_run_preserves_a_concurrent_policy_edit(self):
        source = self.root / "executor.json"
        plan = setup.Instance(
            "configs", self.root / "catalog", "unit", self.root / "override", "", {}, {}
        )

        def choose(*_):
            if not source.exists():
                host.put(source, "{}")
            else:
                source.write_text('{"commands":[]}')
            return "1"

        with (
            patch.object(targets, "choice", side_effect=choose),
            patch.object(setup.docker_reader, "installed_release"),
            patch.object(setup, "check_release"),
            patch.object(setup, "prepare", return_value=plan),
            patch.object(host, "unit_file") as unit,
            self.assertRaisesRegex(ValueError, "其他进程修改"),
        ):
            setup.configure(self.root, self.root, self.root)
        unit.assert_not_called()
        self.assertEqual(setup.read_policy(source), {"commands": []})

    def test_json_fields_and_text_choices(self):
        with patch(
            "builtins.input", side_effect=["1", "1", "8", "1", "1", "warn,error"]
        ):
            rules = targets.fields_for('{"workers":4,"level":"info"}')["fields"]
        self.assertEqual(
            rules,
            {
                "workers": {"min": 1, "max": 8},
                "level": {"choices": ["info", "warn", "error"]},
            },
        )
        with patch("builtins.input", side_effect=["info", "warn"]):
            rules = targets.fields_for("level=info\n")
        self.assertEqual(rules["template"], "level={{value}}\n")
        with self.assertRaisesRegex(ValueError, "模板标记"):
            targets.fields_for("level={{other}}\n")

    def test_existing_container_is_bound_by_engine_and_full_id(self):
        ident = "a" * 64
        with (
            patch.object(
                targets, "docker", side_effect=["local-engine", f"example {ident}"]
            ),
            patch("builtins.input", side_effect=["1", "1"]),
        ):
            self.assertEqual(
                targets.container_target({}),
                {"engine_id": "local-engine", "name": "example", "existing_id": ident},
            )

    def test_new_container_profile_and_mounts(self):
        image = "example/app@sha256:" + "a" * 64
        with (
            patch.object(
                targets,
                "docker",
                side_effect=["engine", "", image, "data\nbackup", '{"/data":{}}'],
            ),
            patch(
                "builtins.input",
                side_effect=[
                    "2",
                    "sample",
                    "1",
                    "8080:80",
                    "1",
                    "2",
                    "2",
                    "/snapshot",
                    "0",
                ],
            ),
        ):
            profile = targets.container_target({})["profiles"]["standard"]
        self.assertEqual(profile.get("ports", []), [])
        self.assertEqual(profile["port_choices"], [[8080, 80]])
        self.assertEqual(
            profile["mounts"], [{"destination": "/data", "volume": "data"}]
        )
        self.assertEqual(
            profile["mount_choices"]["mount-1"],
            {"destination": "/snapshot", "volume": "backup"},
        )
        for invalid in ["0:80", "80:70000", "80/udp", "80:80,80:80"]:
            with self.assertRaises(ValueError):
                targets.ports(invalid)
        self.assertFalse(targets.destination("/data/../etc"))

    def test_existing_override_may_only_gain_approved_app_directories(self):
        override = self.root / "override.conf"
        catalog = {"app": {"path": "/etc/example/settings.json"}}
        with patch.object(targets, "config_file", side_effect=Path):
            old, new = setup.service_override("configs", catalog, override)
            host.put(override, old, 0o644)
            self.assertIn("ReadWritePaths=/etc/example\n", new)
            catalog["other"] = {"path": "/etc/example2/settings.json"}
            before, after = setup.service_override("configs", catalog, override)
            self.assertEqual(before, new)
            self.assertIn("ReadWritePaths=/etc/example2\n", after)
            override.write_text(old + "ReadWritePaths=/\n")
            with self.assertRaisesRegex(ValueError, "自定义服务权限"):
                setup.service_override("configs", catalog, override)

    def test_probe_requires_visible_and_readable_targets(self):
        with (
            patch.object(
                docker_probe,
                "query",
                side_effect=[
                    {"result": {"keys": ["demo"]}},
                    {"error": "target_changed"},
                ],
            ),
            self.assertRaisesRegex(ValueError, "demo"),
        ):
            docker_probe.managed("socket", ["demo"])
