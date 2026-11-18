use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var_os("ASTERION_ACCOUNT_OPERATOR_SOCKET")
        .map(PathBuf::from)
        .ok_or("未配置 ASTERION_ACCOUNT_OPERATOR_SOCKET")?;
    let policy = std::env::var_os("ASTERION_ACCOUNT_OPERATOR_CONFIG_FILE")
        .map(PathBuf::from)
        .ok_or("未配置 ASTERION_ACCOUNT_OPERATOR_CONFIG_FILE")?;
    asterion_executor::serve_account_operator(&address, &policy).await?;
    Ok(())
}
