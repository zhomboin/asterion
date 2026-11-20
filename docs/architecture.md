# 工程架构

浏览器通过回环 HTTP 地址连接本机 Rust 程序。程序校验请求目标和浏览器来源，管理登录会话，把节点、指标和任务记录写入本机 SQLite 数据库。

每个节点有独立的任务队列。本机程序通过 OpenSSH 连接服务器，把采样和查询请求交给 SSH 账号执行。直连节点有 Python 3.9 或以上版本时可读取完整指标；基础采样也可用标准 Linux 命令读取内核数据。进程以 PID 和启动时钟识别，容器以 Docker Engine ID 和完整容器 ID 识别，systemd 服务按 Unit 名识别。

面板校验管理员会话和 CSRF，记录经确认的操作与结果。交互终端通过 SSH PTY 工作，并绑定到打开它的面板会话。退出本机程序后暂停采集。

[连接与采集](monitoring-and-operations.md) · [专用面板](managed-operations.md) · [SQLite 存储](adr/003-sqlite.md)
