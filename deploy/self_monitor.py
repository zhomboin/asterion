"""中心服务器首次部署时，登记本机 Agent 并核对采样。"""

import argparse
import fcntl
import http.cookiejar
import json
import os
import re
import sqlite3
import ssl
import time
import urllib.parse
import urllib.request
from pathlib import Path

NODE_NAME = "中心服务器"
BOOT_DIR = Path("/run/asterion-self-enrollment")
PASSWORD = Path("/run/asterion-first-admin")
CA = Path("/etc/asterion/deploy-ca.crt")
IDENTITY = Path("/var/lib/asterion-agent/identity.json")
DATABASE = Path("/var/lib/asterion/asterion.db")


def register(port, username):
    if not 1 <= port <= 65535:
        raise SystemExit("中心端口无效")
    os.umask(0o077)
    BOOT_DIR.mkdir(mode=0o700, exist_ok=True)
    token_file = BOOT_DIR / "token"
    with (BOOT_DIR / "register.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if token_file.exists():
            if not re.fullmatch(r"[0-9a-f]{64}", token_file.read_text().strip()):
                raise SystemExit("已有注册文件不完整，请核对待接入节点")
            return
        context = ssl.create_default_context(cafile=CA)
        client = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
            urllib.request.HTTPSHandler(context=context),
        )

        def request(path, payload=None):
            url = f"https://127.0.0.1:{port}/api/v1/{path}"
            data = None if payload is None else json.dumps(payload).encode()
            headers = {"Content-Type": "application/json", "X-Asterion-CSRF": "1"}
            req = urllib.request.Request(url, data=data, headers=headers)
            with client.open(req, timeout=8) as response:
                return json.load(response) if response.status != 204 else None

        password = PASSWORD.read_text().rstrip("\n")
        request("auth/login", {"username": username, "password": password})
        try:
            query = urllib.parse.urlencode({"q": NODE_NAME, "limit": 100})
            page = request("nodes?" + query)
            if page["has_more"] or any(
                node["name"] == NODE_NAME for node in page["nodes"]
            ):
                raise SystemExit("已有同名节点或结果不唯一，请在面板核对现有记录")
            issued = request("nodes/enrollment-token", {"name": NODE_NAME, "kind": "linux"})
            if not re.fullmatch(r"[0-9a-f]{64}", issued["token"]):
                raise SystemExit("中心返回的注册凭证格式无效")
            with token_file.open("x") as stream:
                print(issued["token"], file=stream)
        finally:
            request("auth/logout", {})


def wait_for_sample():
    since = int(time.time())
    query = """
        SELECT 1 FROM nodes n JOIN metric_samples s ON s.node_id = n.id
        WHERE n.id = ? AND n.name = ?
          AND n.last_seen_at > ? AND s.observed_at > ? LIMIT 1
    """
    with sqlite3.connect(DATABASE.as_uri() + "?mode=ro", uri=True) as conn:
        for _ in range(90):
            if IDENTITY.is_file():
                node = json.loads(IDENTITY.read_text()).get("node_id")
                if conn.execute(query, (node, NODE_NAME, since, since)).fetchone():
                    print("中心服务器已上报心跳和主机指标")
                    return
            time.sleep(1)
    raise SystemExit("未收到新的自监控数据，请查看 asterion-agent.service 日志")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    enrollment = commands.add_parser("register", help="在回环 HTTPS 中心登记本机")
    enrollment.add_argument("--port", type=int, required=True)
    enrollment.add_argument("--username", required=True)
    commands.add_parser("wait", help="等待本机新的心跳和指标")
    args = parser.parse_args()
    if args.command == "register":
        register(args.port, args.username)
    else:
        wait_for_sample()
