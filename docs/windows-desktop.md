# Windows 原生桌面版

Asterion 桌面版用于连接 Linux 节点，查看 CPU、内存与根分区使用率，打开 SSH 交互终端。窗口使用 iced，SSH 使用 Rust 客户端。运行桌面程序无需浏览器、WSL 或本地 SSH 服务。

## 部署和使用

1. 在 GitHub Actions 的「Windows 桌面版」工作流选择通过检查的运行，下载 `asterion-desktop-windows-x64`。在 Windows x64 电脑的 PowerShell 中解压到自己的应用目录。

   ```powershell
   Expand-Archive -LiteralPath '<下载的 ZIP 完整路径>' -DestinationPath '<应用目录>'
   Set-Location -LiteralPath '<应用目录>'
   .\asterion-desktop.exe
   ```

2. 确认目标 Linux 服务器已启用 SSH，准备有登录权限的普通账号。CPU、内存和根分区采样使用服务器已有的 Linux 命令。打开应用，填写主机地址、端口、SSH 用户名。选择密码，或填写 Windows 私钥文件的完整路径和该私钥的解锁口令。
3. 首次连接时，在服务器控制台或经管理员取得对应主机公钥的 SHA256 指纹。例如，对 Ed25519 主机密钥在服务器执行以下命令。与窗口显示的指纹逐字核对，再点「信任并连接」。指纹改变会阻止连接。

   ```sh
   ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256
   ```

4. 在节点详情查看利用率和趋势。应用约每五秒采样一次，每个节点保存最近 120 个采样。打开终端后在底部输入，支持中文、方向键、Ctrl+C、复制屏幕和粘贴。含换行或制表符的粘贴需要确认。终端空闲两分钟或会话满十五分钟会自动关闭。使用「关闭终端」可继续查看监控。「断开连接」返回节点选择。关闭应用窗口会结束本机会话。
5. 应用仅在内存中使用密码和私钥口令。已连接节点、已信任指纹和采样保存在当前 Windows 用户的应用数据目录下。备份时先退出应用，再在 PowerShell 中复制数据目录。

   ```powershell
   $data = Join-Path $env:LOCALAPPDATA 'Asterion'
   Copy-Item -LiteralPath $data -Destination '<新的备份目录>' -Recurse
   ```

6. 更新时退出应用，将新的下载包解压到新目录后启动。默认沿用当前 Windows 用户的数据。要使用隔离数据目录，启动前在同一 PowerShell 设置 `$env:ASTERION_DESKTOP_DATA_DIR = '<数据目录的完整路径>'`。Linux 面板及远端采集器的部署见[部署指南](deployment.md)。

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
   cargo build --locked --release
   .\target\release\asterion-desktop.exe
   ```

4. 用自己的测试节点验证指纹确认、错误密码、私钥、采样和终端。运行 `top`，调整窗口大小，按 `q` 返回，再验证中文输入、复制和多行粘贴确认。退出后检查桌面进程已结束。测试账号、密钥、地址和数据库只放在私有临时目录。
