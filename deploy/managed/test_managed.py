import json
import socket
import socketserver
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import console
import containers
import host
from common import Refused, digest, fixed_path, revision
from service import Handler, Operator


class ConfigTests(unittest.TestCase):
    def test_schema_rejects_unknown_changed_nested_fields_and_duplicate_keys(self):
        original = '{"workers":2,"fixed":{"enabled":true}}'
        rules = {"workers": {"min": 1, "max": 4}}
        host.validate_config(original, original.replace("2", "4"), rules)
        for data in (original.replace("2", "5"), original.replace("2", "true"),
                     original.replace("true", "1"), '{"workers":2,"workers":3}', '{"workers":2}'):
            with self.subTest(data=data), self.assertRaises(Refused):
                host.validate_config(original, data, rules)

    def test_text_template_does_not_accept_extra_directives(self):
        spec = {"template": "workers {{count}};\n", "fields": {"count": {"choices": ["1", "2"]}}}
        host.validate_candidate(None, spec, b"", b"workers 2;\n")
        with self.assertRaises(Refused):
            host.validate_candidate(None, spec, b"", b"workers 2;\nother;\n")

    def test_failed_reload_restores_content_mode_and_keeps_private_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            path = state / "settings.json"
            old, new = b'{"workers":1}', b'{"workers":2}'
            path.write_bytes(old)
            path.chmod(0o640)
            spec = {"path": str(path), "unit": "fixture.service", "reload": "reload",
                    "fields": {"workers": {"min": 1, "max": 4}}}
            action = {"revision": revision(spec, digest(old)), "digest": digest(new)}
            with patch.object(host, "fixed_path", return_value=path), patch.object(
                host, "command", side_effect=[(1, ""), (0, ""), (0, "")]
            ) as invoke:
                outcome = host.save("fixture", spec, action, new.decode(), state)
                self.assertEqual(invoke.call_count, 3)
                self.assertEqual(outcome["exit_code"], 1)
                self.assertIn("已恢复", outcome["output"])
                self.assertEqual(path.read_bytes(), old)
                self.assertEqual(path.stat().st_mode & 0o777, 0o640)
                self.assertEqual((state / "fixture.backup").stat().st_mode & 0o777, 0o600)
                path.write_bytes(new)
                with self.assertRaises(Refused):
                    host.save("fixture", spec, action, new.decode(), state)
                self.assertEqual(invoke.call_count, 3)

    def test_symlink_is_not_an_editable_file(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "target"
            target.touch()
            link = Path(directory) / "link"
            link.symlink_to(target)
            with self.assertRaises(Refused):
                fixed_path(link)


class PackageTests(unittest.TestCase):
    def test_installed_package_is_not_reinstalled(self):
        with patch.object(host, "command", return_value=(0, "install ok installed")) as invoke:
            spec = {"package": "fixture"}
            value = host.install("app", spec, {"revision": revision("app", spec)})
            self.assertEqual(value["exit_code"], 0)
            self.assertEqual(invoke.call_count, 1)
            self.assertNotIn("apt-get", invoke.call_args.args[0][0])

    def test_refuses_nonliteral_names_before_invoking_package_manager(self):
        with patch.object(host, "command") as invoke:
            for name in ("-y", "pkg-", "./pkg", "name.*", "test=1", "a b"):
                with self.assertRaises(Refused):
                    host.package_view("app", {"package": name})
            invoke.assert_not_called()


class ContainerTests(unittest.TestCase):
    def test_choices_only_select_approved_resources(self):
        mount = {"source": "/example", "destination": "/config"}
        profile = {"restart": "no", "port_choices": [[8080, 80]], "mount_choices": {"conf": mount}}
        selected = {"restart": "no", "ports": [[8080, 80]], "mounts": ["conf"]}
        self.assertEqual(containers.configured(profile, selected)["mounts"], [mount])
        for change in ({"ports": [[22, 22]]}, {"mounts": ["unknown"]}, {"restart": "always"},
                       {"mounts": ["conf", "conf"]}, {"ports": [[8080, 80], [8080, 80]]}):
            with self.assertRaises(Refused):
                containers.configured(profile, {**selected, **change})

    def test_wrong_engine_and_name_never_grant_control(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(containers, "checked", return_value="other"):
            with self.assertRaises(Refused):
                containers.binding("app", {"name": "app", "engine_id": "correct"}, Path(directory))
        with patch.object(containers, "checked", return_value="engine"), patch.object(
                containers, "inspect", return_value="wrong /app"):
            with self.assertRaises(Refused):
                containers.binding("app", {"name": "app", "engine_id": "engine", "existing_id": "a" * 64}, Path("."))

    def test_create_rejects_unpinned_images_and_undeclared_volumes(self):
        profile = {"image": "example:latest", "restart": "no"}
        with self.assertRaises(Refused):
            containers.creation("app", profile, "app")
        profile["image"] = "fixture@sha256:" + "a" * 64
        with patch.object(containers, "checked", return_value='{"/data":{}}'):
            with self.assertRaises(Refused):
                containers.creation("app", profile, "app")
        with patch.object(containers, "checked", return_value="null"):
            args = containers.creation("app", profile, "app")
            self.assertIn("--pull=never", args)
            self.assertIn("--cap-drop=ALL", args)

    def test_recreate_failure_restores_old_binding(self):
        candidate = "c" * 64
        old = "a" * 64
        target = {"id": old, "profile": "small", "engine": "fixture"}
        current = {"revision": "unchanged", "target": target, "running": True, "engine": "fixture"}
        spec = {"name": "app", "profiles": {"large": {}}}
        action = {"verb": "recreate", "revision": "unchanged", "profile": "large"}
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            binding = state / "app.json"
            binding.write_text(json.dumps(target))
            with patch.object(containers, "view", return_value=current), patch.object(
                containers, "creation", return_value=["create"]
            ), patch.object(containers, "checked", side_effect=[
                candidate, "", "", "", Refused(), "", "", ""
            ]) as calls:
                outcome = containers.apply("app", spec, action, state)
                self.assertEqual(outcome["exit_code"], 1)
                self.assertEqual(calls.call_args.args, ("container", "start", "--", old))
                self.assertEqual(json.loads(binding.read_bytes()), target)

    def test_invalid_console_size_rejected_before_io(self):
        shell = console.Shell.__new__(console.Shell)
        with self.assertRaises(Refused):
            shell.resize(10000, 24)

    def test_console_rejects_privileged_container_and_wrong_id(self):
        spec = {"terminal": True, "engine_id": "engine"}
        request = {"key": "app", "engine": "engine", "id": "a" * 64}
        with patch.object(containers, "binding", return_value={"id": request["id"]}), patch.object(
            containers, "inspect", return_value='true\n{"Privileged":true}\n[]'
        ):
            for target in (request, {**request, "id": "b" * 64}):
                with self.assertRaises(Refused):
                    containers.console_target("app", spec, target, None)


class GatewayTests(unittest.TestCase):
    def test_socket_roundtrip_and_change_confirmation(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            path = state / "fixture.json"
            path.write_text('{"workers":1}')
            catalog = {"app": {"path": str(path), "unit": "fixture.service", "reload": "reload",
                               "fields": {"workers": {"min": 1, "max": 2}}}}
            address = str(state / "fixture.sock")
            with socketserver.UnixStreamServer(address, Handler) as server, patch.object(
                    host, "fixed_path", return_value=path), patch.object(host, "command", return_value=(0, "")):
                server.operator = Operator("configs", catalog, state)

                def exchange(payload):
                    worker = threading.Thread(target=server.handle_request)
                    worker.start()
                    try:
                        with socket.socket(socket.AF_UNIX) as client:
                            client.settimeout(2)
                            client.connect(address)
                            client.sendall(json.dumps(payload).encode() + b"\n")
                            with client.makefile("rb") as response:
                                return json.loads(response.readline())
                    finally:
                        worker.join(timeout=2)
                        self.assertFalse(worker.is_alive())

                current = exchange({"query": True, "key": "app"})["result"]
                content = '{"workers":2}'
                request = {"change": {"key": "app", "verb": "save_reload", "revision": current["revision"],
                                      "digest": digest(content.encode())}, "content": content,
                           "expires_at": int(time.time()) + 60}
                self.assertEqual(exchange(request)["result"]["exit_code"], 0)
                self.assertEqual(path.read_text(), content)
                self.assertEqual(exchange(request), {"error": "target_changed"})

    def test_only_authorized_key_can_be_read(self):
        operator = Operator("configs", {"app": {}}, None)
        self.assertEqual(operator.handle({"query": True}), {"keys": ["app"]})
        for key in ("../etc", "other", ""):
            with self.assertRaises(Refused):
                operator.handle({"query": True, "key": key})


if __name__ == "__main__":
    unittest.main()
