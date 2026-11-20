# 在本机使用 Asterion

面板和历史数据都在自己的电脑上。程序运行时通过 SSH 采集，退出或电脑休眠后暂停采集。以下命令在 Linux 或 Windows 的 WSL Ubuntu 终端中执行。

## 启动面板

1. 下载与本机架构匹配的运行包。GitHub 主分支成功的 [CI 运行](https://github.com/zhomboin/asterion/actions/workflows/ci.yml?query=branch%3Amain) 在 Artifacts 中提供 `asterion-local-linux-x86_64`。从 GitHub 下载时先解开外层 ZIP，得到 `.tar.gz`。本机需要 OpenSSH 客户端。
2. 在自己的 Linux 主目录下准备私有目录，解压并启动。

   ```bash
   mkdir -m 700 -p ~/asterion-local
   cd ~/asterion-local
   tar -xzf '<本机运行包的绝对路径>'
   bash start.sh
   ```

3. 首次在终端设置面板口令。浏览器打开终端显示的本机地址，默认是 `http://127.0.0.1:8080`，账号为 `local`。Windows 浏览器可通过 WSL 的 localhost 转发访问这个地址。端口占用时运行 `ASTERION_LOCAL_PORT=8081 bash start.sh`。

已有项目源码、Rust stable 和 Node.js 24 时，可在项目根目录直接运行 `bash start.sh`。[本地开发](development.md)

## 接入并执行命令

1. 在运行面板的同一个 Linux 用户环境中，确认可以用密钥登录目标机。首次连接时，从可信渠道核对 SSH 主机指纹再确认。加密私钥可使用当前 SSH agent 中已解锁的密钥。

   ```bash
   ssh '<SSH别名或用户@主机>' uptime
   ```

   非默认端口可用 `ssh -p '<端口>' '<用户@主机>' uptime`。在 `~/.ssh/config` 中设置了端口、用户或密钥的别名可直接使用。

2. 登录面板，选「节点 → 接入服务器」。输入刚验证的 SSH 别名、`用户@主机` 或 `用户@主机:端口`。IPv6 带端口写成 `用户@[IPv6地址]:端口`。节点名称可留空。
3. 打开「指标」查看最近采样。在「终端」输入本机面板口令，连接后可直接键入 `uptime`、`df -h` 等命令。主机终端使用已有的非 root SSH 账号，执行权限由服务器决定。
4. Linux 节点有可读的 `/proc` 和常用系统命令即可采集 CPU、内存、根分区和网卡累计流量。目标机有 Python 3.9 或以上版本时，可使用完整采样、进程等专用页。软件安装、配置编辑、容器变更等操作可在本机数据目录的 `profiles.json` 为已有连接指定资源范围。[专用面板配置](managed-operations.md)

## 停止与下次使用

在启动终端按 Ctrl+C。下次从同一目录运行 `bash start.sh`，使用原口令登录即可继续使用已有节点和历史记录。数据库在运行目录的 `.local/panel/`。升级时先退出程序，保留此目录，再解压新包。若要放在其他私有目录，每次启动时设置同一个 `ASTERION_LOCAL_DATA_DIR`。
