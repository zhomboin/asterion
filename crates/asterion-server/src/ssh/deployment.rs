mod channel;
use super::{config::Target, transport};
pub(super) use crate::collector_deploy::{deploy, status};
use asterion_protocol::TaskReply;
use tokio::fs::File;
impl crate::collector_deploy::Target for Target {
    async fn exchange(
        &self,
        command: &str,
        upload: Option<(File, &str, &str)>,
    ) -> anyhow::Result<(i32, String)> {
        channel::call(self, command, upload).await
    }
    async fn verify_sample(&self) -> TaskReply {
        transport::call(
            self,
            "sample",
            serde_json::json!({"stream":"","seq":0}),
            10,
            None,
        )
        .await
    }
}
