use crate::{
    App, Message, Method,
    model::Node,
    ssh::{self, Credentials},
    ssh_config::{self, Config},
};
use anyhow::{Context, Result, ensure};
use iced::Task;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Login,
    Verify,
}
#[derive(Default)]
pub struct Options {
    pub name: String,
    pub profiles: Vec<String>,
    pub profile: Option<Arc<Config>>,
    pub saved: Option<Node>,
    pub auto: bool,
    pub remember: bool,
    pub verify_only: bool,
    pub committing: bool,
    pub rebind: Option<(String, String)>,
}
impl App {
    pub(super) fn restore_choice(&mut self, n: &Node) -> Result<()> {
        self.connection.saved = None;
        self.connection.auto = false;
        self.connection.remember = false;
        self.connection.profile = None;
        self.connection.name.clear();
        self.method = Method::Password;
        if let Some(saved) = self
            .store
            .as_ref()
            .context("本机数据不可用")?
            .saved_login(n)?
        {
            self.method = saved.method;
            self.connection.saved = Some(n.clone());
            self.connection.remember = saved.has_secret;
            self.connection.auto = saved.auto;
            match self.method {
                Method::Key => self.key_path = saved.source,
                Method::Config => self.connection.name = saved.source,
                Method::Password => {}
            }
        }
        Ok(())
    }
    pub(super) fn changed_destination(&mut self) {
        self.connection.saved = None;
        self.connection.profile = None;
        self.connection.auto = false;
        self.connection.remember = false;
        self.secret = Default::default();
        self.verify = None;
    }
    pub(super) fn begin(&mut self, intent: Intent) -> Result<Task<Message>> {
        self.connection.verify_only = intent == Intent::Verify;
        if self.method == Method::Config {
            let name = self.connection.name.trim().to_owned();
            self.busy = true;
            self.notice.clear();
            self.epoch += 1;
            let epoch = self.epoch;
            return Ok(Task::perform(ssh_config::resolve(name), move |r| {
                Message::Resolved(epoch, r.map(Arc::new).map_err(|e| format!("{e:#}")))
            }));
        }
        self.perform_connection()
    }
    pub(super) fn perform_connection(&mut self) -> Result<Task<Message>> {
        let n = self.node()?;
        if self.connection.verify_only {
            self.busy = true;
            self.epoch += 1;
            let epoch = self.epoch;
            return Ok(Task::perform(
                ssh::probe(n, self.connection.profile.as_deref().cloned()),
                move |r| Message::Probed(epoch, r.map_err(|e| format!("{e:#}"))),
            ));
        }
        self.connect()
    }
    pub(super) fn accept_profile(&mut self, profile: Arc<Config>) -> Result<Task<Message>> {
        let node = &profile.node;
        ensure!(
            self.connection.saved.as_ref().is_none_or(|n| n == node),
            "已保存连接的目标发生变化，请重新选择并核对新地址"
        );
        self.host = node.host.clone();
        self.user = node.user.clone();
        self.port = node.port.to_string();
        self.connection.profile = Some(profile);
        self.perform_connection()
    }
    pub(super) fn credentials(&self, node: &Node) -> Result<Credentials> {
        Ok(match self.method {
            Method::Config => Credentials::Config(
                self.connection
                    .profile
                    .clone()
                    .context("请先读取本机 SSH 配置")?,
            ),
            Method::Password => {
                let password = if self.secret.is_empty()
                    && self.connection.remember
                    && self.connection.saved.as_ref() == Some(node)
                {
                    self.store
                        .as_ref()
                        .context("本机数据不可用")?
                        .password(node)?
                } else {
                    self.secret.clone()
                };
                ensure!(!password.is_empty(), "请输入 SSH 密码");
                Credentials::Password(password)
            }
            Method::Key => {
                ensure!(!self.key_path.trim().is_empty(), "请输入私钥文件路径");
                Credentials::Key(self.key_path.trim().into(), self.secret.clone())
            }
        })
    }
    pub(super) fn remember_choice(&self, n: &Node) -> Result<()> {
        let store = self.store.as_ref().context("本机数据不可用")?;
        if self.method == Method::Password
            && self.connection.remember
            && self.secret.is_empty()
            && self.connection.saved.as_ref() == Some(n)
        {
            return store.set_auto(n, self.connection.auto);
        }
        let source = match self.method {
            Method::Config => self.connection.name.trim(),
            Method::Key => self.key_path.trim(),
            Method::Password => "",
        };
        store.save_login(
            n,
            self.method,
            source,
            (self.method == Method::Password && self.connection.remember)
                .then_some(self.secret.as_str()),
        )?;
        store.set_auto(n, self.connection.auto)
    }
}
