"""以 Agent 的服务身份验收 Docker 清单，只输出状态和条数。"""

import json
import socket
import sys
import time


def inventory(path):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(10)
        for attempt in range(25):
            try:
                client.connect(path)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if attempt == 24:
                    raise
                time.sleep(0.2)
        client.sendall(b'{"kind":"docker"}\n')
        with client.makefile("rb") as stream:
            line = stream.readline(256 * 1024)
        if not line.endswith(b"\n"):
            raise ValueError("读取服务返回的清单不完整")
        return json.loads(line)


def verify(path, state):
    response = inventory(path)
    data = response.get("data", {})
    status = data.get("status", response.get("code"))
    if state == "pending" and status in ("not_installed", "unreachable"):
        print("Agent 已连通只读服务。Docker Engine 启动后即可读取容器。")
    elif (
        status == "available"
        and data.get("engine_id")
        and isinstance(data.get("containers"), list)
    ):
        print(f"Docker 只读采集已验证，本次读取 {len(data['containers'])} 个容器。")
    else:
        raise ValueError(
            f"Docker 清单读取失败（{status}），请检查 Engine 状态和读取服务日志"
        )


if __name__ == "__main__":
    try:
        verify(sys.argv[1], sys.argv[2])
    except (OSError, ValueError) as exc:
        sys.exit(f"以 Agent 身份验收 Docker 只读采集失败：{exc}")
