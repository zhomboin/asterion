use super::pages::{Kind, Page, string};
use anyhow::{Context, Result, ensure};
use axum::http::Method;
use iced::widget::text_editor;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Node,
    Asset,
    Rule,
    Command,
    Account,
    File,
    Collector,
    Container,
}
use FormKind::*;
#[derive(Clone)]
pub struct Spec {
    pub kind: FormKind,
    pub data: Value,
    pub mode: String,
}
impl Spec {
    pub fn new(kind: FormKind, data: Value, mode: &str) -> Self {
        Self {
            kind,
            data,
            mode: mode.into(),
        }
    }
}
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    pub choices: Vec<String>,
}
pub struct Form {
    pub spec: Spec,
    pub fields: Vec<Field>,
    pub editor: text_editor::Content,
    pub dirty: bool,
    eol: String,
}
pub struct Write {
    pub method: Method,
    pub path: String,
    pub body: Value,
    pub after: Page,
}
pub enum Submit {
    Write(Write),
    Prepare(Value, Option<String>),
}
impl Form {
    pub fn new(spec: Spec) -> Self {
        let data = spec.data.clone();
        let content = data["content"].as_str().unwrap_or("");
        let eol = match content.find(['\r', '\n']).map(|i| &content[i..]) {
            Some(s) if s.starts_with("\r\n") => "\r\n",
            Some(s) if s.starts_with('\r') => "\r",
            _ => "\n",
        };
        let mut form = Self {
            spec,
            fields: vec![],
            editor: text_editor::Content::with_text(
                &content.replace("\r\n", "\n").replace('\r', "\n"),
            ),
            dirty: false,
            eol: eol.into(),
        };
        let mut add = |key: &str, label: &str, value: String, choices: &[&str]| {
            form.fields.push(Field {
                key: key.into(),
                label: label.into(),
                value,
                choices: choices.iter().map(|s| s.to_string()).collect(),
            })
        };
        match form.spec.kind {
            Node => {
                for (k, l) in [
                    ("name", "节点别名"),
                    ("host_name", "宿主机标识"),
                    ("notes", "备注"),
                ] {
                    add(k, l, string(&data, k), &[]);
                }
                add("kind", "系统类型", string(&data, "kind"), &["linux", "wsl"]);
            }
            Asset => {
                for (k, l) in [
                    ("name", "服务名称"),
                    ("description", "用途"),
                    ("repo_url", "仓库地址"),
                    ("deploy_path", "部署目录"),
                    ("config_path", "配置路径"),
                    ("notes", "备注"),
                ] {
                    add(k, l, string(&data, k), &[]);
                }
                add(
                    "management",
                    "管理方式",
                    data["management"].as_str().unwrap_or("other").into(),
                    &["docker", "compose", "systemd", "process", "other"],
                );
                add(
                    "tags",
                    "标签（逗号分隔）",
                    data["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(","),
                    &[],
                );
                add(
                    "verified",
                    "人工核实",
                    if data["verified_at"].is_i64() {
                        "是"
                    } else {
                        "否"
                    }
                    .into(),
                    &["是", "否"],
                );
            }
            Rule => {
                add(
                    "metric",
                    "监控指标",
                    data["metric"].as_str().unwrap_or("cpu").into(),
                    &["offline", "cpu", "memory", "disk"],
                );
                add(
                    "threshold",
                    "阈值（%）",
                    data["threshold"].as_f64().unwrap_or(85.).to_string(),
                    &[],
                );
                add(
                    "for_seconds",
                    "持续时间（秒）",
                    data["for_seconds"].as_i64().unwrap_or(60).to_string(),
                    &[],
                );
                add(
                    "enabled",
                    "启用",
                    if data["enabled"] == false {
                        "否"
                    } else {
                        "是"
                    }
                    .into(),
                    &["是", "否"],
                );
            }
            Command => {
                for p in data["parameters"].as_array().into_iter().flatten() {
                    let k = string(p, "name");
                    let choices = p["values"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>();
                    let label = if p["type"] == "integer" {
                        format!("{k}（{}–{}）", p["min"], p["max"])
                    } else {
                        k.clone()
                    };
                    add(
                        &k,
                        &label,
                        choices.first().copied().unwrap_or("").into(),
                        &choices,
                    );
                }
            }
            Account => {
                let allowed = data["allowed"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                if form.spec.mode == "create" {
                    add(
                        "username",
                        "普通账号名称",
                        allowed.first().copied().unwrap_or("").into(),
                        &allowed,
                    );
                }
                if form.spec.mode == "set_groups" {
                    for group in allowed {
                        let selected = data["groups"]
                            .as_array()
                            .is_some_and(|gs| gs.contains(&json!(group)));
                        add(
                            group,
                            group,
                            if selected { "加入" } else { "不加入" }.into(),
                            &["加入", "不加入"],
                        );
                    }
                }
                if form.spec.mode == "add_key" {
                    add("key", "SSH 公钥", String::new(), &[]);
                }
            }
            Collector => {
                for (k, l, default) in [
                    ("interval_secs", "采样间隔（秒）", 15),
                    ("retention_days", "保留天数", 7),
                    ("max_mib", "缓存上限（MiB）", 128),
                ] {
                    add(
                        k,
                        l,
                        data["settings"][k].as_i64().unwrap_or(default).to_string(),
                        &[],
                    );
                }
            }
            Container => {
                let current = data["target"]["profile"].as_str();
                let options = data["profiles"]
                    .as_object()
                    .into_iter()
                    .flat_map(|o| o.keys())
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                let profile = current
                    .filter(|p| options.contains(p))
                    .or_else(|| options.first().copied())
                    .unwrap_or("");
                add("profile", "配置方案", profile.into(), &options);
                let saved = if current == Some(profile) {
                    &data["target"]["settings"]
                } else {
                    &Value::Null
                };
                form.container_options(profile, saved);
            }
            File => {}
        }
        form
    }
    fn container_options(&mut self, profile: &str, saved: &Value) {
        self.fields.truncate(1);
        let spec = &self.spec.data["profiles"][profile];
        let value = saved["restart"]
            .as_str()
            .or(spec["restart"].as_str())
            .unwrap_or("no")
            .to_owned();
        let mut choices = super::view::items(spec, "restart_choices")
            .iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if choices.is_empty() {
            choices.push(value.clone());
        }
        self.fields.push(Field {
            key: "restart".into(),
            label: "重启策略".into(),
            value,
            choices,
        });
        let mut toggle = |key, label, checked| {
            self.fields.push(Field {
                key,
                label,
                value: if checked { "启用" } else { "停用" }.into(),
                choices: vec!["启用".into(), "停用".into()],
            })
        };
        for pair in super::view::items(spec, "port_choices") {
            let key = format!("{}:{}", pair[0], pair[1]);
            toggle(
                format!("port:{key}"),
                format!("端口 {key}"),
                super::view::items(saved, "ports").contains(pair),
            );
        }
        for (key, mount) in spec["mount_choices"].as_object().into_iter().flatten() {
            let source = mount["volume"]
                .as_str()
                .or(mount["source"].as_str())
                .unwrap_or(key);
            toggle(
                format!("mount:{key}"),
                format!("{source} → {}", string(mount, "destination")),
                super::view::items(saved, "mounts").contains(&json!(key)),
            );
        }
    }
    pub fn change(&mut self, i: usize, value: String) {
        if let Some(f) = self.fields.get_mut(i) {
            let profile = self.spec.kind == Container && f.key == "profile" && f.value != value;
            f.value = value.clone();
            if profile {
                self.container_options(&value, &Value::Null);
            }
            self.dirty = true;
        }
    }
    pub fn get(&self, k: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.key == k)
            .map_or("", |f| &f.value)
    }
    pub fn mixed_eol(&self) -> bool {
        self.spec.data["content"]
            .as_str()
            .unwrap_or("")
            .split(&self.eol)
            .any(|line| line.contains(['\r', '\n']))
    }
    pub fn title(&self) -> &'static str {
        match self.spec.kind {
            Node => "编辑节点资料",
            Asset => "登记服务资产",
            Rule => "编辑告警规则",
            Command => "运行预定义命令",
            Account => "账号变更",
            File => "编辑文件",
            Collector => "部署采集器",
            Container => "容器配置",
        }
    }
    pub fn submit(&self, node: &str) -> Result<Submit> {
        let d = &self.spec.data;
        let id = string(d, "id");
        let g = |k| self.get(k);
        let split = |s: &str| {
            s.split([',', '，'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let nullable = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
        let write = |base: &str, body: Value, page: Kind| {
            let (method, path) = if id.is_empty() {
                (Method::POST, base.to_owned())
            } else {
                (Method::PATCH, format!("{base}/{id}"))
            };
            Submit::Write(Write {
                method,
                path,
                body,
                after: Page::new(page),
            })
        };
        let prepare = |action| Submit::Prepare(action, None);
        if matches!(self.spec.kind, Asset | Rule) {
            ensure!(
                d["node_id"].as_str().filter(|s| !s.is_empty()).is_some() || !node.is_empty(),
                "请选择归属节点"
            );
        }
        Ok(match self.spec.kind {
            Node => write(
                "/nodes",
                json!({"name":g("name"),"host_name":nullable(g("host_name")),"kind":g("kind"),"notes":g("notes")}),
                Kind::Nodes,
            ),
            Asset => write(
                "/assets",
                json!({"node_id":d["node_id"].as_str().unwrap_or(node),"name":g("name"),"management":g("management"),"description":g("description"),"repo_url":nullable(g("repo_url")),"deploy_path":nullable(g("deploy_path")),"config_path":nullable(g("config_path")),"tags":split(g("tags")),"notes":g("notes"),"verified":g("verified")=="是"}),
                Kind::Assets,
            ),
            Rule => write(
                "/alert-rules",
                json!({"node_id":d["node_id"].as_str().unwrap_or(node),"metric":g("metric"),"threshold":if g("metric")=="offline"{None}else{Some(g("threshold").parse::<f64>().context("阈值须为数字")?)},"for_seconds":g("for_seconds").parse::<u32>().context("持续时间须为整数")?,"enabled":g("enabled")=="是"}),
                Kind::Rules,
            ),
            Command => prepare(
                json!({"type":"command","name":d["name"],"parameters":self.fields.iter().map(|f|(f.key.clone(),json!(f.value))).collect::<Map<_,_>>()}),
            ),
            Account => {
                let mut change = json!({"type":self.spec.mode,"username":d["username"].as_str().unwrap_or(g("username"))});
                match self.spec.mode.as_str() {
                    "add_key" => change["key"] = json!(g("key")),
                    "set_groups" => {
                        change["groups"] = json!(
                            self.fields
                                .iter()
                                .filter(|f| f.value == "加入")
                                .map(|f| f.key.as_str())
                                .collect::<Vec<_>>()
                        )
                    }
                    "create" => {}
                    _ => anyhow::bail!("未知账号操作"),
                }
                prepare(json!({"type":"account","change":change}))
            }
            File => {
                let content = self
                    .editor
                    .text()
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', &self.eol);
                ensure!(
                    self.editor.text()
                        != d["content"]
                            .as_str()
                            .unwrap_or("")
                            .replace("\r\n", "\n")
                            .replace('\r', "\n"),
                    "文件内容未修改"
                );
                ensure!(
                    content.len()
                        <= if self.spec.mode == "save_reload" {
                            8192
                        } else {
                            65536
                        },
                    "文件超过大小限制"
                );
                let key = d["key"]
                    .as_str()
                    .or(d["path"].as_str())
                    .context("缺少文件路径")?;
                let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
                Submit::Prepare(
                    json!({"type":"managed","change":{"key":key,"verb":self.spec.mode,"revision":d["revision"],"digest":digest}}),
                    Some(content),
                )
            }
            Collector => {
                let mut settings = json!({});
                for k in ["interval_secs", "retention_days", "max_mib"] {
                    settings[k] = json!(g(k).parse::<u32>().context("部署参数须为正整数")?);
                }
                prepare(
                    json!({"type":"collector_deploy","revision":d["revision"],"settings":settings}),
                )
            }
            Container => {
                let profile = g("profile");
                ensure!(d["profiles"][profile].is_object(), "所选配置方案不可用");
                let restart = g("restart");
                let spec = &d["profiles"][profile];
                let enabled =
                    |prefix: &str, key: &str| self.get(&format!("{prefix}:{key}")) == "启用";
                let ports = super::view::items(spec, "port_choices")
                    .iter()
                    .filter(|v| enabled("port", &format!("{}:{}", v[0], v[1])))
                    .collect::<Vec<_>>();
                let mounts = spec["mount_choices"]
                    .as_object()
                    .into_iter()
                    .flat_map(|o| o.keys())
                    .filter(|k| enabled("mount", k))
                    .collect::<Vec<_>>();
                prepare(
                    json!({"type":"managed","change":{"key":d["key"],"verb":self.spec.mode,"revision":d["revision"],"profile":profile,"settings":{"restart":restart,"ports":ports,"mounts":mounts}}}),
                )
            }
        })
    }
}
