import json
import os
import select
import signal
import sys
import time

from common import Refused


def dispatch(kind, args, policy):
    import monitor

    if kind == "sample":
        return monitor.sample()
    if kind == "processes":
        return monitor.processes(args)
    if kind == "process":
        return monitor.process(args["pid"], detail=True)
    if kind in ("systemd", "systemd_service"):
        return monitor.systemd(args)
    if kind == "terminal_capability":
        return {"enabled": os.geteuid() != 0}
    import actions

    if kind.startswith("docker"):
        import docker_read

        return docker_read.query(kind, args)
    if kind == "accounts":
        return actions.accounts.inventory()
    if kind == "account_keys":
        return actions.accounts.handle(policy.get("accounts", {}), args["username"])
    if kind == "operation_catalog":
        return actions.catalog(policy)
    if kind == "managed_query":
        return actions.managed_query(policy, args)
    if kind == "container_console_target":
        target = args["container"]
        return actions.containers.console_target(
            target["key"],
            actions.entry(policy, "containers", target["key"]),
            target,
            actions.state_dir(),
        )
    if kind == "operation_execute":
        start = time.monotonic()
        value = actions.execute(policy, args)
        value["duration_ms"] = round((time.monotonic() - start) * 1000)
        return value
    raise Refused("unsupported")


def run(request):
    try:
        value = dispatch(request["kind"], request["args"], request.get("policy") or {})
        return dict(outcome="success", value=value)
    except Refused as error:
        code = error.code
    except PermissionError:
        code = "permission_denied"
    except (FileNotFoundError, ProcessLookupError, KeyError):
        code = "not_found"
    except (TypeError, ValueError):
        code = "invalid_argument"
    except Exception:
        code = "execution_failed"
    return dict(outcome="failure", code=code, message="远端请求未完成")


def main(request):
    # 普通查询和命令随 SSH 关闭。已确认的受管变更在时限内完成核验或回滚。
    managed = (
        request["kind"] == "operation_execute"
        and request["args"]["action"]["type"] == "managed"
    )
    pid = os.fork()
    if not pid:
        os.setpgid(0, 0)
        os.umask(0o077)
        print(json.dumps(run(request), ensure_ascii=False), flush=True)
        os._exit(0)
    try:
        os.setpgid(pid, pid)
        end = time.monotonic() + request["seconds"]
        while time.monotonic() < end:
            if os.waitid(os.P_PID, pid, os.WEXITED | os.WNOWAIT | os.WNOHANG):
                break
            if managed:
                time.sleep(0.05)
            elif select.select([sys.stdin], [], [], 0.05)[0] and not os.read(0, 1):
                break
    finally:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(pid, 0)
