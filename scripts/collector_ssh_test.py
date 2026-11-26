"""验证远端采集、SSH 中断和重连补传，进程由调用方统一关闭。"""

import json
import os
from pathlib import Path
import sqlite3
from contextlib import closing

from collector_reader_test import export, verify_reader


def check_collector(work, stack, sshd, node, api, launch, wait, target):
    binary = Path(
        os.environ.get("TEST_COLLECTOR", "target/debug/asterion-collector")
    ).resolve(strict=True)
    bindir = work / "bin"
    bindir.mkdir()
    (bindir / "asterion-collector").symlink_to(binary)
    db = work / "collector.db"
    launch(
        stack,
        [binary, "run"],
        work / "collector.log",
        {
            **os.environ,
            "ASTERION_COLLECTOR_DB": str(db),
            "ASTERION_COLLECTOR_INTERVAL_SECS": "5",
        },
    )
    wait(lambda: api(node + "/metrics").get("collector"), "面板没有通过 SSH 发现采集器")

    # 用一致的快照改变权限，后台采集器继续写入自己的数据库。
    snapshot = work / "reader.db"
    with closing(sqlite3.connect(f"file:{db}?mode=ro", uri=True)) as source:
        with closing(sqlite3.connect(snapshot)) as copy:
            source.backup(copy)
            (stream,) = copy.execute(
                "SELECT stream FROM metadata WHERE id=1"
            ).fetchone()
    snapshot.chmod(0o400)
    batch = verify_reader(work, target, binary, snapshot)
    assert batch["stream"] == stream
    snapshot.chmod(0)
    denied = export(work, target, binary, snapshot)
    assert (
        denied["outcome"] == "failure" and denied["code"] == "collector_unavailable"
    ), denied.get("outcome")
    print("已通过新的 SSH 会话验证只读导出及无读取权限时的错误")

    def readings(after):
        with closing(sqlite3.connect(f"file:{db}?mode=ro", uri=True)) as conn:
            return conn.execute(
                "SELECT seq,captured_at,sample FROM readings WHERE seq>? ORDER BY seq",
                (after,),
            ).fetchall()

    sshd.terminate()
    sshd.wait(timeout=5)
    wait(lambda: api(node)["status"] == "offline", "SSH 断开后节点未显示离线")
    boundary = readings(0)[-1][0]
    wait(lambda: len(readings(boundary)) >= 2, "断开 SSH 后远端未继续采样")
    offline = readings(boundary)
    launch(stack, sshd.args, work / "reconnected-sshd.log")
    node_id = node.rsplit("/", 1)[1]

    def restored():
        with closing(sqlite3.connect(f"file:{work / 'db'}?mode=ro", uri=True)) as conn:
            for _, at, sample in offline:
                row = conn.execute(
                    "SELECT sample FROM metric_samples WHERE node_id=? AND observed_at=?",
                    (node_id, at),
                ).fetchone()
                if row is None or json.loads(row[0]) != json.loads(sample):
                    return False
            return True

    wait(restored, "面板未按原时间补回断线期间的样本")
    assert api(node)["status"] == "online"
    assert all(row in readings(boundary) for row in offline)
    print("已验证真实 SSH 断开期间连续采集、原时间补传和远端副本保留")
