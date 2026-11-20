# 本机运行包的更新、备份与恢复

Asterion 在自己的 Linux 或 WSL 电脑上运行，远端服务器只需接受 SSH 连接。首次下载、解压、启动和添加节点，按[本机使用](local.md)操作。

## 更新运行包

1. 在启动面板的终端按 Ctrl+C，等待进程退出。确认当前运行目录和数据目录。如果启动时设置过 `ASTERION_LOCAL_DATA_DIR`，请使用该目录的实际绝对路径。以下命令在同一个 Linux 终端执行。

   ```bash
   current='<当前运行目录的绝对路径>'
   data="$current/.local/panel"
   test -f "$data/panel.db"
   ```

2. 确认上一步找到数据库后，备份整个数据目录。备份包含面板账号、节点和历史记录，也可能包含连接策略或密钥，存放在自己的私有目录。

   ```bash
   umask 077
   backup=$(mktemp -d "$HOME/asterion-backup.XXXXXXXX")
   cp -a -- "$data" "$backup/panel"
   test -f "$backup/panel/panel.db"
   ```

3. 将新的运行包解压到一个新目录，指定原数据目录启动。用原账号登录，核对已有节点、历史记录和最近采样时间。

   ```bash
   next=$(mktemp -d "$HOME/asterion-local.XXXXXXXX")
   tar -xzf '<新运行包的绝对路径>' -C "$next"
   ASTERION_LOCAL_DATA_DIR="$data" bash "$next/start.sh"
   ```

以后使用同一个绝对数据路径和新版启动脚本。SSH 别名、主机指纹和系统私钥由当前 Linux 用户的 OpenSSH 管理。

## 从备份恢复

退出面板后，将备份复制到一个新目录，以便保留当前数据供核对。使用与备份匹配的运行包启动，登录后抽查一个节点的历史和 SSH 状态。确认恢复结果后，后续继续使用恢复目录。

```bash
umask 077
restore=$(mktemp -d "$HOME/asterion-restore.XXXXXXXX")
cp -a -- '<备份目录的绝对路径>/panel/.' "$restore/"
test -f "$restore/panel.db"
ASTERION_LOCAL_DATA_DIR="$restore" bash '<匹配的运行目录>/start.sh'
```
