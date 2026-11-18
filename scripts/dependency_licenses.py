"""输出锁定的第三方依赖与许可证，缺少许可证声明时报告失败。"""

import json
import subprocess
import sys
from pathlib import Path


def dependencies():
    command = ["cargo", "+stable", "metadata", "--locked", "--format-version", "1"]
    if "--offline" in sys.argv[1:]:
        command.append("--offline")
    metadata = json.loads(subprocess.check_output(command))
    for package in metadata["packages"]:
        if package["source"] is not None:
            license_name = package["license"]
            if not license_name and package["license_file"]:
                license_name = "许可证文件：" + Path(package["license_file"]).name
            yield "cargo", package["name"], package["version"], license_name

    lock = json.loads(Path("web/package-lock.json").read_text(encoding="utf-8"))
    for path, package in lock["packages"].items():
        if path:
            yield "npm", path.rsplit("node_modules/", 1)[-1], package["version"], package.get("license")


entries = sorted(set(dependencies()), key=lambda entry: (*entry[:3], entry[3] or ""))
print("来源\t包名\t版本\t许可证")
missing = []
for ecosystem, name, version, license_name in entries:
    print(ecosystem, name, version, license_name or "缺失", sep="\t")
    if not license_name:
        missing.append(f"{ecosystem}/{name}@{version}")

if missing:
    print("缺少许可证声明：" + ", ".join(missing), file=sys.stderr)
    sys.exit(1)
