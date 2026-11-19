# 节点操作

监控和操作分开授权。在节点详情选择「软件安装」「服务配置」或 Docker 页的「容器服务」。systemd 清单中的启停仍使用执行策略内的 Unit 白名单及主机自己的授权。

## 本地运维验证

在项目根目录的 WSL Bash 中，使用 Python 3、Rust stable 和 Node.js 24。日常开发先运行不接触真实包管理器、服务和容器的用例：

```bash
python3 -B -m unittest discover -s deploy/managed -v
cargo +stable test --locked -p asterion-server --lib operations
cargo +stable check --locked -p asterion-agent --all-targets
npm --prefix web run check
npm --prefix web run build
```

按[本地开发](development.md)启动中心和 Agent；只读节点应显示未授权。要做真实验收，使用可恢复的测试虚拟机，按下方步骤启用所需实例。不要用开发机的业务服务验证停止或重建。测试策略、账号、节点信息只放在忽略的 `.local/` 或目标机私有目录中。

## 远程启用

1. 按[远程部署](deployment.md)发布中心及 Agent。目标机需要 Python 3.10 以上；安装软件要求 Debian/Ubuntu、可信的软件源及当前的包索引；容器操作要求可用的 Docker CLI 和本机 Unix Socket。下列命令在目标 Ubuntu 的 Bash 中执行，每次只选用一个实例。

```bash
kind='<packages、configs 或 containers 三选一>'
case "$kind" in packages|configs|containers) ;; *) exit 1 ;; esac
release=/opt/asterion-agent/current
getent group "asterion-ops-$kind" >/dev/null || sudo groupadd --system "asterion-ops-$kind"
sudo install -m 0644 "$release/deploy/asterion-managed@.service" /etc/systemd/system/
sudo install -m 0600 "$release/deploy/managed-$kind.example.json" "/etc/asterion-agent/managed-$kind.json"
sudoedit "/etc/asterion-agent/managed-$kind.json"
```

2. 替换策略中的所有占位符，删除未授权对象。顶层名称就是面板的目标清单。配置仅限可向面板管理员完整披露的文件，不能选择含密码、令牌、私钥的文件，也不要登记脚本、Unit、sudoers 或可执行配置。配置文件和各级目录必须归 root 或操作身份所有，其他账号不可写；不跟随软链接。

- **软件**：每项填写一个确切的仓库包名。面板显示命令和实际安装状态。不会更新软件源、移除已有包或自动重试失败任务。安装及依赖脚本有系统权限，需单独审核软件源和包。锁等待 30 秒，安装最多约 15 分钟。
- **配置**：每个目标绑定绝对路径、Unit 和明确的 `reload` 或 `restart`。默认校验现有 JSON 对象，只允许修改列出的枚举或整数范围字段。文本文件可使用 `template` 给出完整模板，`{{字段名}}` 仅允许替换为对应 `choices`；可选 `validate` 为节点管理员固定的程序和参数数组，恰好一个 `{candidate}` 参数指向候选文件。校验器必须能独立校验候选文件。配置上限 8192 字节。对不支持重载的服务显式选择重启。
- **容器**：每个服务指定一个 Engine ID、名称及允许的镜像方案。镜像以摘要固定，须预先存在于节点；命名卷预先建立。不指定任意宿主机路径，不开放特权模式。必选项写入 `ports` 和 `mounts`；可选端口对、重启策略和具名挂载分别写入 `port_choices`、`restart_choices`、`mount_choices`，面板只能从中选择。端口对格式为两个整数（主机、容器），仅发布到回环地址。宿主配置文件只读，数据使用命名卷。容器内的目标路径不可重复。如仅对已有容器启停或进入终端，填入 `existing_id`（完整容器 ID）并移除 `profiles`；这种模式不允许重建。

3. 给所选实例配置必要权限，**不要提升 Agent 或宿主机终端身份**。执行以下命令编辑实例覆盖文件，在其 `[Service]` 下填写对应设置：

```bash
sudo systemctl edit "asterion-managed@$kind.service"
```

| 实例 | 实例覆盖文件设置 |
| --- | --- |
| packages | 分行设置 `DynamicUser=no`、`User=root`、`ProtectSystem=no`、空值 `RestrictAddressFamilies=`，再设置 `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK`。系统包安装需要修改系统及联网下载。 |
| configs | 分行设置 `DynamicUser=no`、`User=root`，并逐项增加 `ReadWritePaths=<指定应用的独立配置目录>`。不要给整个系统配置目录写权限。 |
| containers | 使用独立非 root 身份，设置 `SupplementaryGroups=docker`，不共享宿主机终端账号。访问 Docker Socket 相当于获得高权限。 |

4. 按[受控任务](deployment.md#受控任务)配置 Agent 的 `executor.json` 和凭据加载。在其 `managed` 对象中只保留所选能力到 `/run/asterion-managed-<kind>/requests.sock` 的对应关系。给 Agent 的 `[Service]` 增加 `SupplementaryGroups=asterion-ops-<kind>`，将占位符替换为启用的实例名；不同实例不共享访问组。不要将 Agent 加入 docker 组。先核对策略和进程身份，然后启动：

```bash
sudoedit /etc/asterion-agent/executor.json
sudo systemctl edit asterion-agent.service
sudo systemctl daemon-reload
sudo systemd-analyze verify "asterion-managed@$kind.service"
sudo systemctl enable --now "asterion-managed@$kind.service"
sudo systemctl restart asterion-agent.service
sudo systemctl status --no-pager "asterion-managed@$kind.service"
```

5. **可选容器终端**：仅为可信容器显式设置 `terminal: true`。镜像须包含 `/bin/sh` 和 `/usr/bin/timeout`，并允许 UID/GID 65534 使用。不要给挂有主机管理 Socket、主机敏感目录或运行在特权模式的旧容器开启终端。在中心私网部署中启用 `ASTERION_TERMINAL_ENABLED=1`；Agent 的环境文件设置 `ASTERION_CONTAINER_TERMINAL_ENABLED=1` 及 `ASTERION_CONTAINER_TERMINAL_SOCKET=/run/asterion-managed-containers/console.sock`。无需启用节点终端。在各自节点执行：

```bash
# 在中心
sudoedit /etc/asterion/server.env
sudo systemctl restart asterion-server.service
# 在 Agent 主机
sudoedit /etc/asterion-agent/agent.env
sudo systemctl restart asterion-managed@containers.service asterion-agent.service
```

## 验收与维护

先在测试节点依次验证：已安装包不重复安装；无效配置不写入，外部修改后不能覆盖；服务应用失败时恢复原文件；创建的容器先保持停止，启动后才能进入终端；重建前后核对新旧 ID、运行状态和数据卷。不接管同名容器。重建不复制容器可写层。容器输入输出不进入任务记录。容器终端按会话限制时间，不是后台作业管理器。

软件安装、配置应用和容器变更下发后不支持强制取消。浏览器可以关闭；节点断连或中心重启后不要重发，先重新查询目标状态。配置备份及容器绑定在对应实例的 `StateDirectory` 内，限制为操作服务身份访问。维护前分别备份这些目录、应用数据及节点策略；需要原文件时先比较备份，再由管理员恢复并验证目标服务。若操作时节点掉电或 Docker 不可达，检查实际容器、数据卷以及绑定后再恢复，不能凭同名重建绑定。

升级已启用实例时，保留策略、覆盖文件和状态，更新发布包及模板后按顺序执行。卸载前确认无活动安装或服务变更，停用所选实例，再按部署文档归档；不要删除配置备份、容器或卷。

```bash
sudo install -m 0644 /opt/asterion-agent/current/deploy/asterion-managed@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl try-restart "asterion-managed@$kind.service"
```

仅在确认卸载实例时执行：

```bash
sudo systemctl disable --now "asterion-managed@$kind.service"
```
