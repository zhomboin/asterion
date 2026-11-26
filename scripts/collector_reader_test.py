"""从节点 SSH 登录会话验收数据库权限与采样导出。"""

import json
import shlex
import subprocess


def node_command(work, target, command, data=b""):
    return subprocess.check_output(
        [
            "ssh",
            "-F",
            "/dev/null",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            f"UserKnownHostsFile={work / 'known_hosts'}",
            "-i",
            str(work / target["identity_file"]),
            "-p",
            str(target["port"]),
            "-l",
            target["user"],
            target["host"],
            command,
        ],
        input=data,
        timeout=15,
    )


def export(work, target, binary, database=None):
    args = [str(binary), "export"]
    if database is not None:
        args = ["env", f"ASTERION_COLLECTOR_DB={database}", *args]
    command = shlex.join(args)
    return json.loads(
        node_command(work, target, command, b'{"args":{"stream":"","seq":0}}\n')
    )


def verify_reader(work, target, binary, database=None):
    path = shlex.quote(str(database or "/var/lib/asterion-collector/history.db"))
    node_command(
        work,
        target,
        f"test -r {path} && test ! -w {path} || {{ id >&2; namei -l {path} >&2; exit 1; }}",
    )
    reply = export(work, target, binary, database)
    assert reply["outcome"] == "success", reply.get("code")
    batch = reply["value"]["collector"]
    assert batch["records"] and batch["stream"], "SSH 账号未导出采样"
    return batch
