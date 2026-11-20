"""以 Agent 的服务身份验收 Docker 清单，只输出状态和条数。"""

import json
import socket
import sys
import time


def query(path, request):
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
        client.sendall(json.dumps(request).encode() + b"\n")
        with client.makefile("rb") as stream:
            line = stream.readline(256 * 1024)
        if not line.endswith(b"\n"):
            raise ValueError("读取服务返回的清单不完整")
        return json.loads(line)


def inventory(path):
    return query(path, {"kind": "docker"})


def managed(path, keys):
    if query(path, {"query": True}).get("result", {}).get("keys") != keys:
        raise ValueError("操作服务尚未加载预期的授权清单")
    for key in keys:
        result = query(path, {"query": True, "key": key})
        if result.get("result", {}).get("key") != key:
            raise ValueError(f"操作服务无法读取授权目标 {key}，请核对文件或容器身份")


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
        if sys.argv[2] == "managed":
            managed(sys.argv[1], sys.argv[3:])
        else:
            verify(sys.argv[1], sys.argv[2])
    except (OSError, ValueError) as exc:
        sys.exit(f"以 Agent 身份验收节点服务失败：{exc}")
