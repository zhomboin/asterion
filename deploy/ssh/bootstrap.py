"""从一次 SSH 请求装载中心随请求提供的模块。"""

import json
import sys
import types

request = json.loads(sys.stdin.buffer.readline(524288))
for name, source in request.pop("modules"):
    module = types.ModuleType(name)
    sys.modules[name] = module
    exec(compile(source, name, "exec"), module.__dict__)
sys.modules["dispatch"].main(request)
