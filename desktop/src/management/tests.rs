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
