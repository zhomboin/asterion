"""复用内部 CA、TLS 证书和手动部署的暂存文件，按需签发缺少的部分。"""

import getpass
import secrets
import shutil
import subprocess
import tempfile
from pathlib import Path

from existing import trusted
from host import put, run
from inputs import ask


def ca_password(new):
    if not new:
        return getpass.getpass("已有内部 CA 私钥口令：") + "\n"
    while True:
        value = ask(
            "设置内部 CA 私钥口令（至少 12 位）",
            lambda s: len(s) >= 12 and s.strip(),
            secret=True,
        )
        if value == getpass.getpass("确认内部 CA 口令："):
            return value + "\n"
        print("两次输入不同。")


def ensure_tls(config, setup, work):
    trusted(config, directory=True)
    trusted(config / "pki", directory=True)
    trusted(setup, directory=True, private=True)
    names = {
        "ca.key": config / "pki/ca.key",
        "ca.crt": config / "deploy-ca.crt",
        "tls.key": config / "tls.key",
        "tls.crt": config / "tls.crt",
    }
    with tempfile.TemporaryDirectory(dir=work) as tmp:
        stage = Path(tmp)
        for name, dest in names.items():
            private = name.endswith(".key")
            if trusted(dest, private=private):
                shutil.copyfile(dest, stage / name)
            elif name != "ca.key" and trusted(setup / name, private=private):
                shutil.copyfile(setup / name, stage / name)

        def exists(name):
            return (stage / name).exists()

        def openssl(*args, **kwargs):
            return run("openssl", *args, cwd=stage, **kwargs)

        need_ca = not exists("ca.crt")
        need_leaf = not exists("tls.crt")
        if (exists("ca.crt") and not exists("ca.key") and need_leaf) or (
            not exists("tls.key") and not need_leaf
        ):
            raise ValueError("已有证书缺少配对私钥，请核对原密钥位置")
        if need_ca and not exists("ca.key") and not need_leaf:
            raise ValueError("已有服务证书缺少对应的内部 CA")
        secret = ca_password(not exists("ca.key")) if need_ca or need_leaf else None
        if need_ca:
            if not exists("ca.key"):
                openssl(
                    "genrsa",
                    "-aes256",
                    "-passout",
                    "stdin",
                    "-out",
                    "ca.key",
                    "3072",
                    input=secret,
                )
            openssl(
                "req",
                "-x509",
                "-new",
                "-sha256",
                "-days",
                "1825",
                "-key",
                "ca.key",
                "-passin",
                "stdin",
                "-subj",
                "/CN=Asterion Internal CA",
                "-addext",
                "basicConstraints=critical,CA:TRUE,pathlen:0",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
                "-out",
                "ca.crt",
                input=secret,
            )
        if need_leaf:
            if not exists("tls.key"):
                openssl("genrsa", "-out", "tls.key", "3072")
            openssl(
                "req",
                "-new",
                "-key",
                "tls.key",
                "-subj",
                "/CN=localhost",
                "-out",
                "tls.csr",
                input="",
            )
            put(
                stage / "tls.ext",
                "basicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n"
                "keyUsage=critical,digitalSignature,keyEncipherment\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n",
            )
            openssl(
                "x509",
                "-req",
                "-sha256",
                "-days",
                "180",
                "-in",
                "tls.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-passin",
                "stdin",
                "-set_serial",
                f"0x01{secrets.token_hex(15)}",
                "-extfile",
                "tls.ext",
                "-out",
                "tls.crt",
                input=secret,
            )
        for flag, target in (
            ("-verify_hostname", "localhost"),
            ("-verify_ip", "127.0.0.1"),
        ):
            openssl(
                "verify",
                "-purpose",
                "sslserver",
                flag,
                target,
                "-CAfile",
                "ca.crt",
                "tls.crt",
            )
        cert_key = openssl(
            "x509", "-in", "tls.crt", "-noout", "-pubkey", stdout=subprocess.PIPE
        ).stdout
        actual_key = openssl(
            "pkey", "-in", "tls.key", "-pubout", input="", stdout=subprocess.PIPE
        ).stdout
        if cert_key != actual_key:
            raise ValueError("现有中心 TLS 私钥与证书不配对")
        # 已匹配的完整组合通过验证后才发布。已有文件的内容和权限均保留。
        (config / "pki").mkdir(mode=0o700, exist_ok=True)
        for name, dest in names.items():
            if exists(name) and not dest.exists():
                put(
                    dest,
                    (stage / name).read_text(),
                    0o600 if name.endswith(".key") else 0o644,
                )
