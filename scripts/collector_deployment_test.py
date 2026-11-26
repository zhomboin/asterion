"""通过面板实际使用的接口核对部署确认、任务结果与远端服务。"""

import json
import secrets
import time
import urllib.error

from collector_reader_test import node_command, verify_reader


def check_deployment(api, node, password, work, install, target):
    status_path = node + "/collector-deployment"
    status = api(status_path)
    assert status["artifact_available"] and len(status["revision"]) == 64, status
    action = {"type": "collector_deploy", "revision": status["revision"]}
    assert status["state"] == "missing", status

    def prepare(body):
        result = api(node + "/operations", body)
        assert result["step_up_required"]
        return "/api/v1/operations/" + result["task_id"], result["challenge"]

    def rejected(path, body, expected):
        try:
            api(path, body)
        except urllib.error.HTTPError as exc:
            assert exc.code == expected, exc.code
        else:
            raise AssertionError("无效请求被接受")

    def finish(path):
        end = time.monotonic() + 120
        while time.monotonic() < end:
            job = api(path)
            if job["finished_at"]:
                return job
            time.sleep(0.4)
        raise AssertionError("部署任务未完成")

    rejected(
        node + "/operations",
        {**action, "settings": {"interval_secs": 4, "retention_days": 1, "max_mib": 1}},
        400,
    )
    stale, token = prepare({**action, "revision": "0" * 64})
    rejected(stale + "/confirm", {"challenge": token}, 403)
    rejected(
        stale + "/confirm",
        {"challenge": token, "password": password, "sudo_password": "bad\ninput"},
        400,
    )
    api(stale + "/confirm", {"challenge": token, "password": password})
    failed = finish(stale)
    assert failed["state"] == "failed" and "重新检查" in failed["output"], failed
    if not install:
        return

    settings = {"interval_secs": 5, "retention_days": 3, "max_mib": 32}
    marker = secrets.token_hex(20)
    for options in ({"settings": settings}, {}):
        task, token = prepare({**action, **options})
        api(
            task + "/confirm",
            {
                "challenge": token,
                "password": password,
                "sudo_password": marker if options else None,
            },
        )
        job = finish(task)
        assert job["state"] == "succeeded", job
        assert marker not in json.dumps(job)
        installed = api(status_path)
        assert installed["state"] == "active" and installed["enabled"]
        assert installed["settings"] == settings
        node_command(work, target, "systemctl is-enabled --quiet asterion-collector")
        config = node_command(
            work, target, "cat /etc/default/asterion-collector"
        ).decode()
        assert "ASTERION_COLLECTOR_RETENTION_DAYS=3" in config
        assert "ASTERION_COLLECTOR_MAX_MIB=32" in config
        assert "ASTERION_COLLECTOR_INTERVAL_SECS=5" in config
        batch = verify_reader(work, target, "/usr/local/bin/asterion-collector")
        stream = batch["stream"]
        if options:
            original = stream
        else:
            assert stream == original
    for filename in ("db", "db-wal", "center.log", "sshd.log"):
        f = work / filename
        if f.exists():
            assert marker.encode() not in f.read_bytes()
    print(
        "已通过面板确认部署、systemd 开机启动、只读采样及重复部署保留配置与数据库验收"
    )
