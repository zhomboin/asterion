use crate::model::Node;
use anyhow::{Context, Result, ensure};
#[cfg(windows)]
pub fn arguments(line: &str) -> Result<Vec<String>> {
    use windows_sys::Win32::{Foundation::LocalFree, UI::Shell::CommandLineToArgvW};
    ensure!(!line.contains('\0'), "参数包含空字符");
    let wide: Vec<_> = line.trim().encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    // CommandLineToArgvW 拥有所有分配的参数，复制后统一归还给 Windows。
    unsafe {
        let args = CommandLineToArgvW(wide.as_ptr(), &mut count);
        ensure!(!args.is_null(), "无法解析 Windows 程序参数");
        let args_slice = std::slice::from_raw_parts(args, count as usize);
        let mut out = Vec::new();
        for &arg in args_slice {
            let mut len = 0;
            while *arg.add(len) != 0 {
                len += 1;
            }
            out.push(String::from_utf16_lossy(std::slice::from_raw_parts(
                arg, len,
            )));
        }
        LocalFree(args.cast());
        Ok(out)
    }
}
#[cfg(not(windows))]
pub fn arguments(line: &str) -> Result<Vec<String>> {
    shlex::split(line).context("程序参数引号不完整")
}
pub fn proxy(line: &str, node: &Node, alias: &str) -> Result<tokio::process::Command> {
    let args = arguments(line)?;
    let (exe, params) = args.split_first().context("代理命令缺少程序名")?;
    ensure!(
        !exe.is_empty() && !exe.contains('%'),
        "代理程序名不能引用连接参数"
    );
    let mut cmd = tokio::process::Command::new(exe);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    for value in params {
        let mut chars = value.chars();
        let mut out = String::new();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => out.push('%'),
                Some('n') => out.push_str(alias),
                Some('h') => out.push_str(&node.host),
                Some('p') => out.push_str(&node.port.to_string()),
                Some('r') => {
                    ensure!(
                        asterion_server::connection_name::valid_profile(&node.user),
                        "代理用户名格式无效"
                    );
                    out.push_str(&node.user);
                }
                _ => anyhow::bail!("不支持的代理参数占位符"),
            }
        }
        cmd.arg(out);
    }
    cmd.kill_on_drop(true);
    Ok(cmd)
}
pub fn web_address(value: &str) -> bool {
    value.parse::<axum::http::Uri>().is_ok_and(|u| {
        matches!(u.scheme_str(), Some("http" | "https")) && u.host().is_some_and(|s| !s.is_empty())
    })
}
pub fn open_repository(value: &str) -> Result<()> {
    ensure!(web_address(value), "仓库地址须为 HTTP 或 HTTPS");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let value: Vec<_> = value.encode_utf16().chain(Some(0)).collect();
        let verb: Vec<_> = "open".encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        ensure!(result as isize > 32, "无法打开系统浏览器");
    }
    #[cfg(not(windows))]
    {
        let mut child = std::process::Command::new("xdg-open").arg(value).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_substitutes_only_arguments_and_repository_links_require_web_schemes() {
        let node = Node::parse("node.invalid", "2222", "operator").unwrap();
        let cmd = proxy(
            "relay --dest %h:%p --name %n --user %r %%",
            &node,
            "example",
        )
        .unwrap();
        assert_eq!(
            cmd.as_std().get_args().collect::<Vec<_>>(),
            [
                "--dest",
                "node.invalid:2222",
                "--name",
                "example",
                "--user",
                "operator",
                "%"
            ]
        );
        assert!(proxy("%h %p", &node, "example").is_err());
        assert!(proxy("relay %x", &node, "example").is_err());
        assert!(web_address("https://example.invalid/project"));
        assert!(!web_address("file:///local"));
    }
}
