# ADR-005 systemd 读取方式

Agent 通过固定路径的 `systemctl` 向本机 systemd 读取 Service Unit。支持在节点上按 Unit 名过滤，返回 Active、Sub、描述与可取得的属性。读取参数来自固定列表，运行时限制输出和等待时间。路径与启动信息附来源和观测时间。状态区分未启用 systemd、访问被拒绝和查询失败；一个 Unit 处于 inactive 是可观测状态。

服务启停和重启通过受控执行器。由节点管理员配置可操作的具体 Unit，中心确认目标后下发动作，Agent 以进程现有权限执行。读取与变更使用各自受限的命令入口。
