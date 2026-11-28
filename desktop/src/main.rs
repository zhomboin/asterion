#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod chart;
mod desk;
mod keys;
mod management;
mod model;
mod screen;
mod ssh;
mod store;
mod terminal;
mod ui;
mod workspace;

use iced::{Font, Size, Subscription, Task, Theme, widget::operation};
use model::{Node, Sample};
use ssh::{Credentials, Session};
use std::{path::PathBuf, time::Duration};
use store::Store;
use terminal::{Event, Input};
use tokio::sync::mpsc;
use zeroize::{Zeroize, Zeroizing};

fn main() -> iced::Result {
    iced::application(App::new, App::update, ui::view)
        .title("Asterion · 节点工作台")
        .theme(|_: &App| Theme::Light)
        .default_font(Font::with_name("Microsoft YaHei UI"))
        .window(iced::window::Settings {
            size: Size::new(1060., 790.),
            min_size: Some(Size::new(780., 620.)),
            ..Default::default()
        })
        .subscription(App::subscription)
        .run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Password,
    Key,
}
#[derive(Clone)]
enum Message {
    Desk(desk::Action),
    Idle,
    Focus,
    Host(String),
    Port(String),
    User(String),
    Secret(String),
    KeyPath(String),
    Method(Method),
    Select(Node),
    Connect,
    Trust,
    Cancel,
    Login(u64, Result<ssh::Login, String>),
    Tick,
    Sample(u64, Result<Sample, String>),
    Open,
    Close,
    Term(u64, Event),
    Text(String),
    Key(Vec<u8>),
    Geometry(u16, u16),
    Copy,
    Paste,
    Clipboard(u64, Option<String>),
    PasteConfirm,
    PasteCancel,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", std::mem::discriminant(self))
    }
}

struct App {
    desk: desk::Desk,
    store: Option<Store>,
    nodes: Vec<Node>,
    host: String,
    port: String,
    user: String,
    secret: Zeroizing<String>,
    key_path: String,
    method: Method,
    notice: String,
    busy: bool,
    verify: Option<String>,
    epoch: u64,
    session: Option<Session>,
    node_id: Option<i64>,
    history: Vec<Sample>,
    sampling: bool,
    metrics_error: String,
    term_epoch: u64,
    input: Option<mpsc::Sender<Input>>,
    ready: bool,
    screen_cache: iced::widget::canvas::Cache,
    editor: iced::widget::text_editor::Content,
    parser: vt100::Parser,
    geometry: (u16, u16),
    term_status: String,
    paste: Option<String>,
}

impl App {
    fn new() -> Self {
        let store = Store::open();
        let mut notice = String::new();
        let store = store.map_err(|e| notice = format!("{e:#}")).ok();
        let nodes = store
            .as_ref()
            .map(|s| {
                s.nodes().unwrap_or_else(|e| {
                    notice = format!("无法读取节点：{e}");
                    Vec::new()
                })
            })
            .unwrap_or_default();
        let first = nodes.first();
        Self {
            desk: Default::default(),
            host: first.map(|n| n.host.clone()).unwrap_or_default(),
            port: first
                .map(|n| n.port.to_string())
                .unwrap_or_else(|| "22".into()),
            user: first.map(|n| n.user.clone()).unwrap_or_default(),
            store,
            nodes,
            notice,
            secret: Zeroizing::default(),
            key_path: String::new(),
            method: Method::Password,
            busy: false,
            verify: None,
            epoch: 0,
            session: None,
            node_id: None,
            history: Vec::new(),
            sampling: false,
            metrics_error: String::new(),
            term_epoch: 0,
            input: None,
            ready: false,
            screen_cache: Default::default(),
            editor: Default::default(),
            parser: vt100::Parser::new(12, 80, 0),
            geometry: (80, 12),
            term_status: String::new(),
            paste: None,
        }
    }

    fn node(&self) -> anyhow::Result<Node> {
        Node::parse(&self.host, &self.port, &self.user)
    }
    fn connect(&mut self) -> anyhow::Result<Task<Message>> {
        let node = self.node()?;
        let expected = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("本机数据不可用"))?
            .fingerprint(&node)?;
        let auth = match self.method {
            Method::Password => {
                anyhow::ensure!(!self.secret.is_empty(), "请输入 SSH 密码");
                Credentials::Password(self.secret.clone())
            }
            Method::Key => {
                anyhow::ensure!(!self.key_path.trim().is_empty(), "请输入私钥文件路径");
                Credentials::Key(PathBuf::from(self.key_path.trim()), self.secret.clone())
            }
        };
        self.busy = true;
        self.notice.clear();
        self.epoch += 1;
        let epoch = self.epoch;
        Ok(Task::perform(ssh::login(node, expected, auth), move |r| {
            Message::Login(epoch, r.map_err(|e| format!("{e:#}")))
        }))
    }

    fn poll(&mut self) -> Task<Message> {
        let Some(session) = self.session.clone() else {
            return Task::none();
        };
        if self.sampling {
            return Task::none();
        }
        self.sampling = true;
        let epoch = self.epoch;
        Task::perform(
            async move {
                tokio::time::timeout(Duration::from_secs(16), ssh::sample(session))
                    .await
                    .map_err(|_| "采样超时".to_owned())?
                    .map_err(|e| format!("{e:#}"))
            },
            move |r| Message::Sample(epoch, r),
        )
    }

    fn close_term(&mut self) {
        self.term_epoch += 1;
        self.input = None;
        self.ready = false;
        self.paste = None;
        self.term_status = "终端已关闭".into();
    }

    fn send(&mut self, input: Input) {
        if let Some(tx) = &self.input
            && tx.try_send(input).is_err()
        {
            self.term_status = "终端繁忙或已断开，请重试".into();
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match self.handle(msg) {
            Ok(t) => t,
            Err(e) => {
                self.notice = format!("{e:#}");
                Task::none()
            }
        }
    }

    fn handle(&mut self, msg: Message) -> anyhow::Result<Task<Message>> {
        match msg {
            Message::Desk(action) => return self.desktop(action),
            Message::Host(v) => self.host = v,
            Message::Port(v) => self.port = v,
            Message::User(v) => self.user = v,
            Message::Secret(v) => self.secret = Zeroizing::new(v),
            Message::KeyPath(v) => self.key_path = v,
            Message::Method(m) => {
                self.method = m;
                self.secret.zeroize();
            }
            Message::Select(n) => {
                self.host = n.host;
                self.port = n.port.to_string();
                self.user = n.user;
                self.secret.zeroize();
                self.key_path.clear();
                self.notice.clear();
            }
            Message::Connect if !self.busy && self.session.is_none() && self.verify.is_none() => {
                return self.connect();
            }
            Message::Trust => {
                if let Some(key) = self.verify.take() {
                    self.store.as_ref().unwrap().trust(&self.node()?, &key)?;
                    return self.connect();
                }
            }
            Message::Cancel => {
                self.close_term();
                if let Ok(node) = self.node() {
                    self.desk.active.remove(&workspace::Workspace::id(&node));
                }
                let session = self.session.take();
                self.epoch += 1;
                self.verify = None;
                self.busy = false;
                self.secret.zeroize();
                self.node_id = None;
                self.history.clear();
                self.sampling = false;
                self.notice.clear();
                self.metrics_error.clear();
                if let Some(session) = session {
                    return Ok(Task::perform(
                        async move {
                            let _ = session
                                .disconnect(russh::Disconnect::ByApplication, "", "")
                                .await;
                        },
                        |_| Message::Idle,
                    ));
                }
            }
            Message::Login(epoch, r) if self.epoch == epoch => {
                self.busy = false;
                match r {
                    Ok(ssh::Login::Verify(key)) => self.verify = Some(key),
                    Ok(ssh::Login::Connected(session)) => {
                        self.secret.zeroize();
                        let node = self.node()?;
                        let db = self.store.as_ref().unwrap();
                        let id = db.connected(&node)?;
                        self.history = db.history(id)?;
                        self.nodes = db.nodes()?;
                        self.session = Some(session);
                        self.node_id = Some(id);
                        self.notice.clear();
                        self.term_status.clear();
                        return Ok(Task::batch([self.poll(), self.attach_workspace()?]));
                    }
                    Err(e) => {
                        self.secret.zeroize();
                        self.notice = e
                    }
                }
            }
            Message::Tick => {
                let refresh = self.refresh_management();
                return Ok(Task::batch([self.poll(), refresh]));
            }
            Message::Sample(epoch, r) if epoch == self.epoch => {
                self.sampling = false;
                match r {
                    Ok(s) => {
                        let id = self.node_id.unwrap();
                        let db = self.store.as_mut().unwrap();
                        db.save(id, &s)?;
                        self.history = db.history(id)?;
                        self.metrics_error.clear();
                    }
                    Err(e) => self.metrics_error = e,
                }
            }
            Message::Open if self.input.is_none() => {
                if let Some(session) = self.session.clone() {
                    self.term_epoch += 1;
                    let epoch = self.term_epoch;
                    let (tx, rx) = mpsc::channel(64);
                    self.input = Some(tx);
                    let (c, r) = self.geometry;
                    self.parser = vt100::Parser::new(r, c, 0);
                    self.screen_cache.clear();
                    self.ready = false;
                    self.term_status = "正在打开终端…".into();
                    return Ok(terminal::run(session, c, r, rx, self.desk.container.take())
                        .map(move |e| Message::Term(epoch, e)));
                }
            }
            Message::Close => self.close_term(),
            Message::Focus if self.ready && self.paste.is_none() => {
                return Ok(operation::focus("terminal-input"));
            }
            Message::Term(epoch, event) if self.term_epoch == epoch => match event {
                Event::Open => {
                    self.ready = true;
                    self.term_status.clear();
                    let (c, r) = self.geometry;
                    self.send(Input::Size(c, r));
                    return Ok(operation::focus("terminal-input"));
                }
                Event::Bytes(bytes) => {
                    self.parser.process(&bytes);
                    self.screen_cache.clear();
                }
                Event::End(reason) => {
                    self.close_term();
                    self.term_status = reason;
                }
            },
            Message::Geometry(c, r) => {
                self.geometry = (c, r);
                self.parser.screen_mut().set_size(r, c);
                self.screen_cache.clear();
                if self.ready {
                    self.send(Input::Size(c, r));
                }
            }
            Message::Text(s) if self.ready && self.paste.is_none() && !s.is_empty() => {
                if s.len() <= 4096 && !s.chars().any(char::is_control) {
                    self.send(Input::Bytes(s.into_bytes()));
                }
            }
            Message::Key(b) if self.ready && self.paste.is_none() => self.send(Input::Bytes(b)),
            Message::Copy => return Ok(iced::clipboard::write(self.parser.screen().contents())),
            Message::Paste if self.ready => {
                let epoch = self.term_epoch;
                return Ok(iced::clipboard::read().map(move |s| Message::Clipboard(epoch, s)));
            }
            Message::Clipboard(e, Some(v)) if e == self.term_epoch && self.ready => {
                let bytes = terminal::paste_bytes(&v, self.parser.screen().bracketed_paste())?;
                if v.contains(['\r', '\n', '\t']) {
                    self.paste = Some(v)
                } else {
                    self.send(Input::Bytes(bytes))
                }
            }
            Message::PasteConfirm => {
                if let Some(v) = self.paste.take() {
                    let bytes = terminal::paste_bytes(&v, self.parser.screen().bracketed_paste())?;
                    self.send(Input::Bytes(bytes));
                    return Ok(operation::focus("terminal-input"));
                }
            }
            Message::PasteCancel => {
                self.paste = None;
                return Ok(operation::focus("terminal-input"));
            }
            _ => {}
        }
        Ok(Task::none())
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.session.is_none() && !self.desk.visible {
            return Subscription::none();
        }
        iced::time::every(Duration::from_secs(5)).map(|_| Message::Tick)
    }
}
