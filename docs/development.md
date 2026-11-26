# 本地开发

在 Linux 或 WSL Ubuntu 的项目根目录执行以下命令。准备 Rust stable、Node.js 24、npm、Linux C 编译工具链和 OpenSSH 客户端。预构建包的使用步骤见[本机使用](local.md)。

## 从源码启动

1. 确认当前终端使用 Linux 工具链。

   ```bash
   cargo --version
   node --version
   npm --version
   ssh -V
   ```

2. 启动面板。脚本准备前端依赖，构建网页和 Rust 程序。后端默认监听 `127.0.0.1:17110`，网页和 API 共用该端口。

   ```bash
   bash start.sh
   ```

3. 首次启动时在终端设置本机面板口令。浏览器打开打印出的地址，使用账号 `local` 登录。在「节点 → 接入服务器」填写主机、端口和用户名，选择认证方式。首次遇到未知主机时，在面板核对指纹后继续。进入「指标」核对采样，在「终端」验证只读命令 `uptime`。
4. 修改前端时，可在另一个终端执行下列命令，随后刷新浏览器。修改 Rust 代码后，在启动终端按 Ctrl+C，再运行 `bash start.sh`。

   ```bash
   npm --prefix web run build -- --watch
   ```

开发数据默认在项目的 `.local/panel/`。实际使用的账号、地址、私钥和测试数据只放在 Git 忽略的私有位置。结束开发时，在面板和前端监听终端分别按 Ctrl+C。

## 本地运行服务器采集器

在项目根目录执行。数据留在 Git 忽略的测试目录。第二个终端在相同目录导出第一批记录，结束时在采集终端按 Ctrl+C。从源码启动面板也会构建同架构采集器。部署到其他架构时，可将兼容的二进制放入面板可执行文件旁的 `collectors/<架构>/asterion-collector`。CI 为正式运行包附带 ARM64 二进制。远端界面部署、SSH 自动同步和保留参数见[服务器端采集器](collector.md)。

```bash
cargo build --locked -p asterion-collector
mkdir -p -m 700 .local/collector-dev
export ASTERION_COLLECTOR_DB="$PWD/.local/collector-dev/history.db"
"${CARGO_TARGET_DIR:-target}/debug/asterion-collector" run
```

```bash
export ASTERION_COLLECTOR_DB="$PWD/.local/collector-dev/history.db"
printf '%s\n' '{"args":{"stream":"","seq":0}}' | "${CARGO_TARGET_DIR:-target}/debug/asterion-collector" export
cargo test --locked -p asterion-collector
cargo test --locked -p asterion-server --lib ssh::collector
```

## 按改动验证

```bash
cargo fmt --all -- --check
cargo test --locked -p asterion-server --lib ssh
python3 -B -m unittest discover -s deploy/ssh
python3 -B -m unittest discover -s deploy/managed
npm --prefix web run check
npm --prefix web test
```

验证 Nginx 配置编辑时，把 `TEST_NGINX` 指向可执行的 Nginx 程序。测试在项目的临时目录启动独立实例，检查引用清单、重载及错误回滚，结束后关闭实例。

```bash
TEST_NGINX="$(command -v nginx)" python3 -B -m unittest discover -s deploy/ssh -p test_text_files.py
```

验证节点认证和文件保存时，先构建后端，再运行 SSH 集成测试。测试使用本机回环端口、临时密钥和一次性密码，需要本机测试环境提供 OpenSSH 服务端程序及 Python Paramiko。

```bash
cargo build --locked -p asterion-server -p asterion-collector
TEST_COLLECTOR="${CARGO_TARGET_DIR:-target}/debug/asterion-collector" \
TEST_CENTER="${CARGO_TARGET_DIR:-target}/debug/asterion-server" /usr/bin/python3 -B scripts/test_ssh.py
```

## 构建和验证本机运行包

```bash
npm --prefix web ci
npm --prefix web run build
cargo build --locked --release -p asterion-server -p asterion-collector
bash scripts/package-local.sh "${CARGO_TARGET_DIR:-target}/release/asterion-server"
python3 -B scripts/test_local_package.py ".local/asterion-local-linux-$(uname -m).tar.gz"
```

最后一条命令在临时目录验证解压、首次设置口令和登录，然后换一个端口重启并再次登录。要验证默认监听地址，请在运行测试前留出 `17110` 端口。GitHub CI 在 Ubuntu 22.04 与 24.04 验收，在 22.04 上构建用于下载的 x86_64 运行包。
