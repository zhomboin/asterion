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

3. 首次在终端设置面板口令。浏览器打开 `http://127.0.0.1:17110`，使用账号 `local` 登录。Windows 浏览器可通过 WSL 的 localhost 转发访问这个地址。若端口已被占用，可运行 `ASTERION_LOCAL_PORT=17111 bash start.sh`，再打开终端显示的地址。

已有项目源码、Rust stable 和 Node.js 24 时，可在项目根目录直接运行 `bash start.sh`。[本地开发](development.md)

## 接入并执行命令

1. 登录面板，打开「节点 → 接入服务器」。填写节点别名、主机地址（IP、域名或 OpenSSH 别名）、远端端口和用户名。端口留空采用本机 SSH 配置，通常为 22。环境、宿主机标识和备注在「更多设置」中。
2. 选择认证方式后点击「接入节点」。

   | 认证方式 | 填写内容 |
   | --- | --- |
   | 服务器密码 | 填远端 SSH 账号的登录密码。密码以明文保存在本机私有数据库中，编辑页不回显。服务器拒绝密码后会暂停重试，在节点的「管理」中重新填写后恢复。 |
   | 指定 SSH 私钥 | 填 Linux 或 WSL 中的私钥路径，例如 `~/.ssh/id_ed25519`。对应公钥应已获远端账号授权。加密私钥先用 `ssh-add '<私钥路径>'` 在当前 SSH agent 中解锁，从同一环境启动面板。 |
   | 使用本机 SSH 配置 | 沿用本机默认密钥、SSH agent 或 `~/.ssh/config`。已有 OpenSSH 别名时，只填主机地址即可。 |

3. 遇到未知主机时，面板显示密钥算法和 SHA256 指纹。与服务器控制台或管理员提供的指纹核对一致后，点击「信任并继续」。取消会返回表单。本机 OpenSSH 已信任的主机直接继续，已记录的主机密钥变更或撤销时会阻止连接。
4. 在节点详情等待状态变为「在线」，打开「指标」核对最近采样。在「终端」输入本机面板口令，连接后可运行 `uptime`、`df -h`。主机终端使用非 root SSH 账号，执行权限由服务器决定。
5. Linux 节点有可读的 `/proc` 和常用系统命令即可采集 CPU、内存、根分区和网卡累计流量。目标机有 Python 3.9 或以上版本时，可使用完整采样、进程等专用页。软件安装、配置编辑、容器变更等操作可在本机数据目录的 `profiles.json` 为已有连接指定资源范围。[专用面板配置](managed-operations.md)

## 停止与下次使用

在启动终端按 Ctrl+C。下次从同一目录运行 `bash start.sh`，使用原口令登录即可继续使用已有节点和历史记录。数据库在运行目录的 `.local/panel/`。若要放在其他私有目录，每次启动时设置同一个 `ASTERION_LOCAL_DATA_DIR`。[本机运行包更新、备份与恢复](deployment.md)
