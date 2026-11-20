"""在一次性 Ubuntu 中验证交互部署及面板的授权目标。"""

import hashlib
import json
import shlex
import tempfile
import time
from pathlib import Path

import docker_probe
import host
import services


def verify_managed(root, run, curl, cookie, api, container):
    units = [f"asterion-managed@{kind}.service" for kind in ("configs", "containers")]
    sample_unit = "asterion-ci-managed-sample.service"
    fragment = host.UNITS / sample_unit
    script = shlex.join(["bash", str(root / "deploy/install-center.sh")])

    def wizard(*answers):
        run(
            "script",
            "-qec",
            script,
            "/dev/null",
            input=("\n".join(answers) + "\n").encode(),
        )

    def read(path):
        return json.loads(
            run(
                *curl,
                "-b",
                cookie,
                "--retry",
                "8",
                "--retry-delay",
                "1",
                api + path,
                capture_output=True,
            ).stdout
        )

    with tempfile.TemporaryDirectory(prefix="asterion-ci-app-", dir="/etc") as tmp:
        path = Path(tmp) / "settings.json"
        host.put(path, '{"workers":2}\n')
        host.put(
            fragment,
            "[Unit]\nDescription=Asterion CI sample\n[Service]\nType=oneshot\nExecStart=/usr/bin/true\nRemainAfterExit=yes\n",
            0o644,
        )
        try:
            run("systemctl", "daemon-reload")
            rows = sorted(
                run(
                    "docker",
                    "ps",
                    "-a",
                    "--no-trunc",
                    "--format",
                    "{{.Names}} {{.ID}}",
                    capture_output=True,
                )
                .stdout.decode()
                .splitlines()
            )
            selected = next(
                str(i) for i, row in enumerate(rows, 1) if row.split()[1] == container
            )
            wizard(
                "4",
                "3",
                "settings",
                str(path),
                sample_unit,
                "restart",
                "1",
                "1",
                "8",
                "0",
                "app",
                "1",
                selected,
                "0",
                "1",
            )
            config = read("/managed/configs?key=settings")
            assert config["path"] == str(path) and config["reload"] == "restart"
            assert read("/managed/containers?key=app")["target"]["id"] == container
            assert sorted(read("/operation-catalog")["managed"]) == [
                "configs",
                "containers",
            ]
            pids = [
                services.property_value(unit, "MainPID")
                for unit in [host.AGENT, *units]
            ]
            wizard("4", "3", "0", "0", "1")
            assert [
                services.property_value(unit, "MainPID")
                for unit in [host.AGENT, *units]
            ] == pids
            # 实际验证配置目录可写且指定 Unit 生效，测试不重启业务服务。
            run("systemctl", "start", sample_unit)
            before = services.property_value(sample_unit, "InvocationID")
            content = '{"workers":3}\n'
            response = docker_probe.query(
                "/run/asterion-managed-configs/requests.sock",
                {
                    "change": {
                        "key": "settings",
                        "verb": "save_reload",
                        "revision": config["revision"],
                        "digest": hashlib.sha256(content.encode()).hexdigest(),
                    },
                    "expires_at": int(time.time()) + 30,
                    "content": content,
                },
            )
            assert response["result"]["exit_code"] == 0 and path.read_text() == content
            assert services.active(sample_unit)
            assert services.property_value(sample_unit, "InvocationID") != before
        finally:
            run("systemctl", "stop", sample_unit)
            fragment.unlink()
            run("systemctl", "daemon-reload")
