use super::{
    forms::{Form, FormKind, Spec, Submit},
    pages::{Kind, Page},
};
use iced::widget::text_editor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn editing_files_preserves_revision_and_original_line_endings() {
    let file =
        json!({"path":"/example/file","content":"one\r\ntwo\r\n","revision":"0123456789abcdef"});
    let mut f = Form::new(Spec::new(FormKind::File, file, "save_text"));
    assert!(f.submit("unused").is_err());
    f.editor = text_editor::Content::with_text("one\nthree\n");
    let Submit::Prepare(action, Some(content)) = f.submit("unused").unwrap() else {
        panic!("缺少确认步骤")
    };
    assert_eq!(content, "one\r\nthree\r\n");
    assert_eq!(
        action["change"]["digest"],
        format!("{:x}", Sha256::digest(content.as_bytes()))
    );
    assert_eq!(action["change"]["revision"], "0123456789abcdef");
    f.editor = text_editor::Content::with_text(&"x".repeat(65537));
    assert!(f.submit("unused").is_err());
}

#[test]
fn discovery_uses_asset_node_and_file_requests_encode_the_entire_path() {
    let page = Page::detail(Kind::Discoveries, "asset", json!({"node":"asset-node"}));
    assert!(
        page.path("other-node", "", 24)
            .starts_with("/nodes/asset-node/discoveries?")
    );
    let p = Page::detail(Kind::Entry, "/dir/one & two", json!("files"));
    assert_eq!(
        p.path("node", "", 0),
        "/nodes/node/managed/files?key=%2Fdir%2Fone%20%26%20two"
    );
    let p = Page::detail(Kind::Processes, "", Value::Null);
    assert!(
        p.path("node", "a&b", 24)
            .contains("q=a%26b&sort=pid&limit=25&offset=24")
    );
}

#[test]
fn recreate_keeps_selected_options_and_resets_them_for_a_different_profile() {
    let settings = json!({"restart":"always","ports":[[8080,80]],"mounts":["data"]});
    let data = json!({"key":"example","revision":"revision","target":{"profile":"web","settings":settings},
        "profiles":{"empty":{"restart":"no"},"web":{"restart":"no","restart_choices":["no","always"],
        "port_choices":[[8080,80],[443,443]],"mount_choices":{"data":{"volume":"data","destination":"/data"}}}}});
    let mut form = Form::new(Spec::new(FormKind::Container, data, "recreate"));
    let Submit::Prepare(action, None) = form.submit("node").unwrap() else {
        panic!("缺少操作")
    };
    assert_eq!(action["change"]["profile"], "web");
    assert_eq!(action["change"]["settings"], settings);
    form.change(0, "empty".into());
    let Submit::Prepare(action, None) = form.submit("node").unwrap() else {
        panic!("缺少操作")
    };
    assert_eq!(
        action["change"]["settings"],
        json!({"restart":"no","ports":[],"mounts":[]})
    );
}

#[test]
fn targets_filters_and_service_alerts_keep_the_selected_node() {
    for kind in [FormKind::Asset, FormKind::Rule] {
        let form = Form::new(Spec::new(
            kind,
            json!({"node_id":"second","name":"示例"}),
            "",
        ));
        let Submit::Write(w) = form.submit("first").unwrap() else {
            panic!("缺少写入")
        };
        assert_eq!(w.body["node_id"], "second");
    }
    let filters = json!({"node_id":"second","tag":"a & b","management":"systemd"});
    assert_eq!(
        Page::detail(Kind::Assets, "", filters.clone()).path("first", "", 24),
        "/assets?limit=24&offset=24&q=&node_id=second&management=systemd&tag=a%20%26%20b"
    );
    assert_eq!(
        Page::detail(Kind::Alerts, "resolved", filters).path("first", "", 0),
        "/alerts?limit=24&offset=0&node_id=second&state=resolved"
    );
    let alert =
        json!({"node_id":"second","node_name":"节点","metric":"service","unit":"example.service"});
    let super::Event::NodePage(id, _, p) = super::selection::alert_target(&alert) else {
        panic!("缺少服务入口")
    };
    assert_eq!(
        p.path(&id, "", 0),
        "/nodes/second/service-monitors/example%2Eservice/history"
    );
    assert_eq!(
        Page::detail(Kind::PinLive, "docker", Value::Null).path("second", "", 12),
        "/nodes/second/managed-resources/docker/live?offset=12"
    );
}

#[test]
fn live_pins_match_resource_identity_and_keep_absent_distinct_from_unknown() {
    use super::registry::observed;
    let process = json!({"target":"worker"});
    let rows =
        json!([{"name":"workers","pid":1},{"name":"worker","pid":2},{"name":"worker","pid":3}]);
    assert_eq!(observed("process", &process, &rows).1.len(), 2);
    let pin = json!({"engine_id":"a","target":"id"});
    let mut live = json!({"status":"available","engine_id":"b","containers":[{"id":"id"}]});
    assert_eq!(observed("docker", &pin, &live).0, "连接了其他引擎");
    live["engine_id"] = json!("a");
    assert_eq!(observed("docker", &pin, &live).1.len(), 1);
    live["containers"] = json!([]);
    assert_eq!(observed("docker", &pin, &live).0, "未发现容器");
    live["truncated"] = json!(true);
    assert_eq!(observed("docker", &pin, &live).0, "状态未知");
}

#[test]
fn account_mutations_require_an_enabled_policy_and_an_allowed_non_system_account() {
    use super::resources::account_editable;
    let cat = json!({"enabled":true,"accounts":{"enabled":true,"usernames":["member","root"]}});
    let user = json!({"username":"member","uid":1001,"system":false});
    assert!(account_editable(&cat, &user));
    for (field, value) in [
        ("uid", json!(0)),
        ("system", json!(true)),
        ("username", json!("root")),
        ("username", json!("unknown")),
    ] {
        let mut denied = user.clone();
        denied[field] = value;
        assert!(!account_editable(&cat, &denied));
    }
    assert!(!account_editable(&Value::Null, &user));
    let file = json!({"content":"one\r\ntwo\n","revision":"fixture","path":"/example"});
    let f = Form::new(Spec::new(FormKind::File, file, "save_text"));
    assert!(f.mixed_eol());
    assert!(f.submit("node").is_err());
}
