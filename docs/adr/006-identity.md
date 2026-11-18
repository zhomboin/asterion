# ADR-006 宿主机、节点和运行环境身份

中心创建不可预测的 128 位节点 ID。节点地址变化时通过持久设备凭证重新绑定同一节点。Agent 身份文件与中心 WSS 地址绑定，各 Agent 使用独立文件。节点名称和 IP 可以变化，不同节点允许同名。

宿主机是可选的逻辑实体。WSL Ubuntu 是 Linux 实例，面板显示 WSL Agent 连通状态与可取得的发行版。在缺少独立 Windows 观测时，Windows 宿主机状态保持未知。

发现记录以节点、运行环境和资源身份为键。Docker 采集优先使用 Engine API 的 Engine ID，另保存不含凭证的端点指纹。访问同一 Docker Desktop Engine 的 WSL 节点各保留观察来源。Engine 与容器的身份使用 Engine ID 加完整容器 ID，端点指纹不足以断定两个节点使用同一 Engine。面板按节点显示观察结果，不跨节点累加容器数据。启停操作再次检查 Engine ID。访问授权见 [ADR-004](004-docker.md)。

进程的资源身份包含 PID 和内核启动时钟，降低 PID 复用带来的错绑风险。systemd 使用具体 Unit 名。人工资产绑定要求观察记录与资产属于同一节点，发现结果不覆盖登记内容。
