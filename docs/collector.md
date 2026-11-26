# 服务器端离线采集器

需要在面板退出、电脑休眠或 SSH 中断时保存主机指标的 Linux 节点，可以安装 `asterion-collector`。systemd 随服务器启动采集器，数据写入服务器的 `/var/lib/asterion-collector/history.db`。面板通过现有 SSH 账号只读拉取，无需新增监听端口。CPU、负载、内存、交换空间、文件系统和网卡在采集范围内。服务器关机、采集器停机期间会留下空档。进程、容器、服务检查和告警仍由在线面板处理。

## 安装到远端服务器

1. 使用与服务器架构和 Linux 运行库兼容的运行包。在本机 Linux 或 WSL 上解压后进入运行包目录。若目标架构没有预构建包，在兼容的 Linux 环境运行 `cargo build --locked --release -p asterion-collector`，二进制在 `target/release/`，部署文件在源码的 `deploy/collector/`。

2. 在本机复制二进制和三个部署文件到服务器的私有临时目录。替换 SSH 目标占位符，首次连接先核实主机指纹。

   ```bash
   target='<服务器 SSH 别名>'
   ssh "$target" 'mkdir -m 700 -p ~/asterion-collector-install'
   scp asterion-collector deploy/collector/asterion-collector.{service,conf,sysusers} "$target:asterion-collector-install/"
   ssh "$target"
   ```

3. 在服务器安装程序、服务和专用账号。将面板实际使用的 SSH 账号加入只读组。数据库目录由 systemd 创建，采集器用户可写，组成员可读。以下示例适用于使用 systemd 的 Ubuntu。更新时替换程序和服务文件，保留已有配置和数据库。

   ```bash
   cd ~/asterion-collector-install
   reader='<面板使用的 SSH 用户名>'
   sudo install -m 0755 asterion-collector /usr/local/bin/asterion-collector
   sudo install -m 0644 asterion-collector.sysusers /etc/sysusers.d/asterion-collector.conf
   sudo systemd-sysusers /etc/sysusers.d/asterion-collector.conf
   sudo usermod -aG asterion-collector "$reader"
   sudo install -m 0644 asterion-collector.service /etc/systemd/system/asterion-collector.service
   if ! sudo test -e /etc/default/asterion-collector; then
       sudo install -m 0644 asterion-collector.conf /etc/default/asterion-collector
   fi
   sudo systemctl daemon-reload
   sudo systemctl enable --now asterion-collector.service
   sudo systemctl restart asterion-collector.service
   systemctl is-active asterion-collector.service
   ```

4. 用面板的 SSH 账号建立新会话，确认非交互式 SSH 可以找到程序并读取缓存。以下命令在本机执行。成功响应含 `collector` 和采样序号。若失败，在服务器检查服务日志、组成员资格和数据库目录权限。

   ```bash
   printf '%s\n' '{"args":{"stream":"","seq":0}}' | ssh '<面板的 SSH 目标>' 'asterion-collector export'
   ```

   ```bash
   journalctl -u asterion-collector.service -n 30 --no-pager
   ```

5. 按[本机使用](local.md)启动面板、接入节点并打开「指标」。出现「服务器采集器」即已开始同步。退出面板，等待至少两个远端采样周期，再启动面板，检查这段时间的曲线。无需手工导入或确认每一批数据。

## 保留时间和清理

| 位置 | 默认值 | 配置与清理 |
| --- | --- | --- |
| 服务器 | 每 15 秒采样，保存 7 天，每节点缓存预算 256 MiB | `ASTERION_COLLECTOR_INTERVAL_SECS` 为 5–3600 秒，`ASTERION_COLLECTOR_RETENTION_DAYS` 为 1–365 天，`ASTERION_COLLECTOR_MAX_MIB` 为 1–16384 MiB。启动和写入前检查，超过期限或容量时分批删除最早的记录。 |
| 面板原始指标 | 7 天 | `ASTERION_METRIC_RETENTION_DAYS` 为 1–365 天，从采样时间起计算。 |
| 面板五分钟趋势 | 90 天 | `ASTERION_TREND_RETENTION_DAYS` 为 30–365 天。已超过原始保留期但仍在趋势窗口内的补传样本只参与汇总。 |

服务器缓存由数据库和短暂的 SQLite 回滚日志共用。为保证写入中断后可恢复，约一半预算预留给日志，所以 256 MiB 并不等于可存放 256 MiB 的样本。清理后的数据库自动收缩。systemd 日志由服务器的 journald 独立管理。面板在启动及运行期间每小时清理本机过期数据。

成功同步后，远端副本仍保留至达到时间或容量限制；多台面板可以各自同步。SSH 恢复后，面板从已提交的序号分批读取，按采样时间写入，数据库事务同时保存样本、趋势和进度。服务器清理掉的未读记录无法补回，节点「指标」显示累计缺失数。保持两端系统时钟同步；领先面板不超过五分钟的样本会等待校时或下一轮同步，超出五分钟的样本计入缺失。

修改服务器配置后重启服务。缩短期限或调小容量会在下次启动时立即清理超限数据。希望覆盖较长离线期时，请同时核对远端容量和面板保留窗口。

```bash
sudoedit /etc/default/asterion-collector
sudo systemctl restart asterion-collector.service
systemctl is-active asterion-collector.service
```
