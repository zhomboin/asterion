# 工程架构

[Windows 客户端](windows-desktop.md)使用 iced 原生窗口和 russh。管理工作区在同一进程调用共享 Rust 业务接口，保留口令、会话、节点策略、任务二次确认、文件版本核对与审计。桌面进程不监听管理端口。每个已连接节点独立采样，同一 SSH 连接可承载多个查询和 PTY。采集器部署后另建连接核验权限。快捷连接与管理工作区分别使用 SQLite，SSH 凭据仅在活动连接的内存中使用。

## Linux 本机面板

浏览器通过回环 HTTP 地址连接本机 Rust 程序。程序校验请求目标和浏览器来源，管理登录会话，把节点、指标和任务记录写入本机 SQLite 数据库。

每个节点有独立的任务队列。本机程序通过 OpenSSH 连接服务器，把采样和查询请求交给 SSH 账号执行。直连节点有 Python 3.9 或以上版本时可读取完整指标；基础采样也可用标准 Linux 命令读取内核数据。进程以 PID 和启动时钟识别，容器以 Docker Engine ID 和完整容器 ID 识别，systemd 服务按 Unit 名识别。

面板校验管理员会话和 CSRF，记录经确认的操作与结果。交互终端通过 SSH PTY 工作，并绑定到打开它的面板会话。面板通过已确认的节点运维任务部署 [Rust 采集器](collector.md)，传输前复核节点和账号，启动 systemd 服务后用新 SSH 会话校验采样。它独立写入节点 SQLite，面板通过现有 SSH 逐批只读导出，在同一事务内保存历史、五分钟趋势和同步进度。

[连接与采集](monitoring-and-operations.md) · [专用面板](managed-operations.md) · [SQLite 存储](adr/003-sqlite.md)
