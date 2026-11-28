# Windows 原生桌面版

Asterion 在 Windows 本机运行原生窗口，通过 SSH 管理 Linux 节点。快捷连接用于查看利用率和打开主机终端；管理工作区提供多节点监控、资产、告警和运维任务。界面、业务处理和数据库均在桌面进程内运行。

## 部署和使用

1. 在 GitHub Actions 的「Windows 桌面版」工作流选择通过检查的运行，下载 `asterion-desktop-windows-x64`。在 Windows x64 电脑的 PowerShell 中解压到自己的应用目录。

   ```powershell
   Expand-Archive -LiteralPath '<下载的 ZIP 完整路径>' -DestinationPath '<应用目录>'
   Set-Location -LiteralPath '<应用目录>'
   .\asterion-desktop.exe
   ```

2. 确认目标 Linux 服务器已启用 SSH，准备有登录权限的普通账号。CPU、内存和根分区采样使用服务器已有的 Linux 命令。打开应用，填写主机地址、端口、SSH 用户名。选择密码或私钥。使用私钥时填写 Windows 文件完整路径及解锁口令。使用「本机 SSH 配置」时，输入 OpenSSH 别名或选择命名连接。该方式需要 Windows 的 OpenSSH 客户端；可在 PowerShell 运行 `Get-Command ssh, ssh-keygen` 检查。
3. 可先点击「核对指纹」，单独检查 SSH 目标。首次连接时，在服务器控制台或经管理员取得对应主机公钥的 SHA256 指纹。例如，对 Ed25519 主机密钥在服务器执行以下命令。与窗口显示的指纹逐字核对，再点「信任并连接」。指纹改变会阻止连接。

   ```sh
   ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256
   ```

4. 在节点详情查看利用率和趋势。应用约每五秒采样一次，每个节点保存最近 120 个采样。打开终端后在底部输入，支持中文、方向键、F1 至 F12、Ctrl+C、滚轮回看、复制屏幕和粘贴。含换行或制表符的粘贴需要确认。终端空闲两分钟或会话满十五分钟会自动关闭。使用「关闭终端」可继续查看监控。「断开连接」返回节点选择。关闭应用窗口会结束本机会话。
5. SSH 密码默认仅用于当前连接。勾选「为此节点保存密码」后，应用将密码绑定到用户、主机和端口，用当前 Windows 用户的 DPAPI 加密保存。可用「清除保存的登录信息」移除。私钥口令和 sudo 密码仅用于当前会话。已连接节点、已信任指纹和采样保存在当前 Windows 用户的应用数据目录下。备份时先退出应用，再在 PowerShell 中复制数据目录。

   ```powershell
   $data = Join-Path $env:LOCALAPPDATA 'Asterion'
   Copy-Item -LiteralPath $data -Destination '<新的备份目录>' -Recurse
   ```

6. 下载包包含桌面程序和旁边的 `collectors` 目录，两者一起解压。更新时退出应用，将新的下载包解压到新目录后启动。默认沿用当前 Windows 用户的数据。要使用隔离数据目录，启动前在同一 PowerShell 设置 `$env:ASTERION_DESKTOP_DATA_DIR = '<数据目录的完整路径>'`。Linux 面板及远端采集器的部署见[部署指南](deployment.md)。

## 使用管理工作区

1. 打开「管理工作区」。首次使用设置并重复输入本机工作区口令，长度为 12 至 128 个 UTF-8 字节。以后用该口令解锁。每次解锁会导入新登记的快捷连接。工作区口令和 SSH 登录凭据各自独立。
2. 在工作区点「SSH 连接」，填写节点并登录，再点「管理工作区」。已连接的节点默认每 15 秒采样。可在连接时勾选「解锁工作区后自动连接」，已解锁的工作区会重试意外断开的连接，指纹冲突或身份验证被拒绝时停止重试。加密私钥需先在本机 SSH Agent 解锁，再用「本机 SSH 配置」自动登录。要同时监控另一台服务器，从工作区再点「SSH 连接」。锁定工作区或退出程序会结束这些连接。离线时可解锁查看历史指标、资产、告警与任务。要迁移节点，先断开该节点，在详情点击「更改 SSH 地址」。核对新指纹并登录后，原节点的历史及关联记录会保留。
3. 在「节点总览」选择节点。点击「主机指标」查看一小时或一天内的 CPU、内存、交换空间、磁盘和网卡。点击「长期趋势」切换七天或三十天，默认保留七天原始指标和九十天聚合数据。进程和 Docker 页面可查看详情、关注资源及读取容器日志。在 systemd 选择具体服务可启用持续监控，状态变化保存在「持续监控」中。
4. 在「服务资产」登记归属、管理方式、部署路径和备注。读取节点的进程、Docker 或 systemd 页面后，可将同一节点的发现记录关联到资产。在「告警规则」给所选节点设置离线或使用率条件，在「告警事件」切换活动和已恢复记录。关闭工作区期间，持续服务检查和告警评估会暂停。
5. 在「运维操作」选择预定义命令、软件、服务配置、容器或文件。文件编辑支持最长 64 KiB 的 UTF-8 文本。Nginx 配置从当前实例的引用清单中选择，保存时执行校验并申请重载。账号与公钥操作在「系统账号」中执行。核对节点、动作和参数后确认，受保护的变更还需输入工作区口令。「任务记录」可查看结果、审计和可取消的任务。文件保存失败时，可在任务详情返回草稿或复制内容。关闭有未保存修改的窗口时可以选择继续编辑。
6. 需在面板关闭期间保留主机指标时，在节点的「采集器部署」查看架构、服务状态和安装文件。选择沿用当前或默认配置，或指定采样间隔、保留天数和缓存上限，再确认部署。普通账号须具备所需 sudo 权限。部署完成后应用会通过独立 SSH 连接验证新采样，恢复连接时自动补传。服务器前提与手动部署命令见[采集器指南](collector.md)。
7. 自定义授权时，在当前用户数据目录创建 UTF-8 的 `policies.json`。顶层键使用「节点资料」显示的完整 SSH 地址，值只包含该节点的 `policy` 对象。可按[策略示例](../deploy/ssh.example.json)逐项添加实际需要的目标。默认可读取监控、打开文件和运行预定义的 uptime。自定义策略中需显式设置 `file_editor` 为 true 才开启文件编辑。策略在后续请求生效。也可使用数据目录的 `profiles.json` 或用 `ASTERION_SSH_CONFIG_FILE` 指定其路径；命名连接使用自身的策略。直连地址同时出现在两份文件时，优先使用 `policies.json` 对应条目。受管容器终端还需要容器策略开启 terminal 并通过远端身份及隔离校验。
8. 当前用户数据目录中的 `desktop.sqlite3` 保存快捷连接、主机指纹和最近采样，`workspace.sqlite3` 保存管理记录与口令哈希。按上面的备份步骤一起复制数据库和策略。可选保存的密码密文位于 `desktop.sqlite3`，恢复到其他 Windows 账号或电脑后需重新输入。

命名连接文件格式与[Linux 连接样例](../deploy/ssh.example.json)一致。私钥及已知主机文件的相对路径以该配置文件所在目录为准。使用自定义 OpenSSH 配置文件时，启动前在 PowerShell 将 `$env:ASTERION_LOCAL_SSH_CONFIG` 设置为文件完整路径。已知主机文件可位于含空格的路径；若 OpenSSH 展开后存在多种实际文件解释，请使用命名连接的 `known_hosts_file` 明确指定。

工作区读取 `ASTERION_SAMPLE_INTERVAL_SECS`、`ASTERION_METRIC_RETENTION_DAYS`、`ASTERION_TREND_RETENTION_DAYS` 与 `ASTERION_TERMINAL_ENABLED`。在启动程序的 PowerShell 中设置相应环境变量，重新启动后从「设置」检查生效值。例如 `$env:ASTERION_TERMINAL_ENABLED = '0'` 会关闭主机和容器交互终端。

[逐项功能对照](desktop-parity.md)

## 在 Windows 本地开发

1. 安装 Rust stable 的 Windows MSVC 工具链和 Visual Studio Build Tools 的「使用 C++ 的桌面开发」组件。在 Windows 本机的开发 PowerShell 打开项目的桌面目录。

   ```powershell
   Set-Location -LiteralPath '<项目目录>\desktop'
   rustup show active-toolchain
   rustc -V
   cargo -V
   ```

2. 使用私有临时目录存放验证数据，然后构建并打开原生窗口。首次构建需要下载 Cargo.lock 中的依赖。

   ```powershell
   $env:ASTERION_DESKTOP_DATA_DIR = '<本地临时配置目录>'
   cargo run --locked
   ```

3. 关闭窗口，在相同目录执行针对桌面代码的检查和发布构建。

   ```powershell
   cargo fmt -- --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked
   $env:RUSTFLAGS = '-C target-feature=+crt-static'
   cargo build --locked --release --target x86_64-pc-windows-msvc
   .\target\x86_64-pc-windows-msvc\release\asterion-desktop.exe
   ```

4. 用自己的测试节点验证指纹确认、错误密码、私钥、采样和终端。运行 `top`，调整窗口大小，按 `q` 返回，再验证中文输入、复制和多行粘贴确认。退出后检查桌面进程已结束。测试账号、密钥、地址和数据库只放在私有临时目录。

5. 本地验证采集器部署时，将已解压发布包的采集器目录放到开发程序旁边，再启动应用。连接可用于验证的 Linux 节点，在「采集器部署」核对架构和匹配文件状态。

   ```powershell
   Copy-Item -LiteralPath '<已解压发布包>\collectors' -Destination '.\target\debug' -Recurse -Force
   cargo run --locked
   ```
