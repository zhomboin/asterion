use super::{Running, Workspace, remote::Remote};
use crate::{
    Method,
    model::Node,
    ssh::{self, Credentials, Login, Session},
    ssh_config,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use std::sync::Arc;
pub type Attachment = (String, Arc<Running>, Remote);
impl Workspace {
    pub async fn resume(&self, node: &Node) -> Result<(Session, Attachment)> {
        self.api(axum::http::Method::GET, "/me", serde_json::Value::Null)
            .await?;
        let store = Store::open_in(&self.directory)?;
        let pin = store.fingerprint(node)?.context("节点尚未核对指纹")?;
        let choice = store.saved_login(node)?.context("未保存登录方式")?;
        ensure!(choice.auto, "此节点未开启恢复连接");
        let auth = match choice.method {
            Method::Password => Credentials::Password(store.password(node)?),
            Method::Key => Credentials::Key(choice.source.into(), Default::default()),
            Method::Config => {
                let config = ssh_config::resolve(choice.source).await?;
                ensure!(&config.node == node, "连接配置对应的目标地址已变化");
                Credentials::Config(Arc::new(config))
            }
        };
        let Login::Connected(session) = ssh::login(node.clone(), Some(pin), auth).await? else {
            anyhow::bail!("主机指纹未确认")
        };
        let attachment = self.attach(node.clone(), session.clone()).await?;
        Ok((session, attachment))
    }
}
