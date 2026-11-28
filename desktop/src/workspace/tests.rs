use super::*;
use asterion_protocol::TaskReply;
use asterion_server::ssh_bus::Control;

#[tokio::test]
async fn workspace_keeps_credentials_local_and_requires_confirmation() -> Result<()> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
    std::fs::create_dir_all(&base)?;
    let dir = tempfile::tempdir_in(base)?;
    let secret = Zeroizing::new(format!("{:032x}", rand::random::<u128>()));
    let password = || secret.clone();
    let node = Node::parse("test.invalid", "22", "operator")?;
    let id = Workspace::id(&node);
    let core = Workspace::open(dir.path().into(), password(), vec![node]).await?;
    assert!(
        Workspace::open(dir.path().into(), password(), vec![])
            .await
            .is_err()
    );
    assert_eq!(
        request(&core.router, "", Method::GET, "/nodes", Value::Null)
            .await?
            .status(),
        401
    );
    let nodes = core
        .api(Method::GET, "/nodes?metrics=true", Value::Null)
        .await?;
    assert_eq!(nodes["nodes"][0]["id"], id);
    let asset = core
        .api(
            Method::POST,
            "/assets",
            json!({"node_id":id,"name":"服务","management":"process"}),
        )
        .await?;
    assert_eq!(asset["node_id"], id);
    core.api(
        Method::POST,
        "/alert-rules",
        json!({"node_id":id,"metric":"offline","threshold":null,"for_seconds":0,"enabled":true}),
    )
    .await?;
    let path = format!("/nodes/{id}/operations");
    let proposal = core
        .api(
            Method::POST,
            &path,
            json!({"type":"command","name":"uptime","parameters":{}}),
        )
        .await?;
    let job = format!("/operations/{}", proposal["task_id"].as_str().unwrap());
    let confirm = format!("{job}/confirm");
    assert_eq!(
        core.api(Method::GET, &job, Value::Null).await?["state"],
        "prepared"
    );
    assert!(
        core.api(Method::POST, &confirm, json!({"challenge":"wrong"}))
            .await
            .is_err()
    );
    let mut sub = core.bus.attach(&id, true);
    let done = tokio::spawn(async move {
        let Control::Task(task) = sub.next().await else {
            panic!("缺少执行请求")
        };
        assert_eq!(task.request.kind, "operation_execute");
        assert_eq!(task.request.args["action"]["name"], "uptime");
        let _ = task.response.send(TaskReply::Success {
            value: json!({"exit_code":0,"output":"完成","truncated":false,"duration_ms":1}),
        });
    });
    let body = json!({"challenge":proposal["challenge"]});
    core.api(Method::POST, &confirm, body.clone()).await?;
    tokio::time::timeout(Duration::from_secs(3), done).await??;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = core.api(Method::GET, &job, Value::Null).await?;
            if result["state"] == "succeeded" {
                assert_eq!(result["output"], "完成");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert!(core.api(Method::POST, &confirm, body).await.is_err());
    core.pool.close().await;
    drop(core);
    assert!(
        Workspace::open(
            dir.path().into(),
            Zeroizing::new("invalid-password".into()),
            vec![]
        )
        .await
        .is_err()
    );
    let again = Workspace::open(dir.path().into(), password(), vec![]).await?;
    assert_eq!(
        again.api(Method::GET, &job, Value::Null).await?["state"],
        "succeeded"
    );
    assert!(
        again
            .api(Method::DELETE, &format!("/nodes/{id}"), Value::Null)
            .await
            .is_err()
    );
    let address = Node::parse("test.invalid", "22", "operator")?;
    let shortcuts = Store::open_in(dir.path())?;
    shortcuts.connected(&address)?;
    shortcuts.trust(&address, "test-fingerprint")?;
    let asset_id = asset["id"].as_str().unwrap();
    again
        .api(Method::DELETE, &format!("/assets/{asset_id}"), Value::Null)
        .await?;
    again
        .api(Method::DELETE, &format!("/nodes/{id}"), Value::Null)
        .await?;
    assert!(shortcuts.nodes()?.is_empty());
    assert!(shortcuts.fingerprint(&address)?.is_some());
    again.pool.close().await;
    Ok(())
}

#[tokio::test]
async fn rebind_preserves_history_and_rejects_conflicts_and_unfinished_jobs() -> Result<()> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
    std::fs::create_dir_all(&base)?;
    let dir = tempfile::tempdir_in(base)?;
    let old = Node::parse("prior.invalid", "22", "member")?;
    let new = Node::parse("next.invalid", "22", "member")?;
    let other = Node::parse("claimed.invalid", "22", "member")?;
    let password = Zeroizing::new(format!("{:032x}", rand::random::<u128>()));
    let mut store = Store::open_in(dir.path())?;
    let old_key = store.connected(&old)?;
    store.trust(&old, "previous-pin")?;
    store.save(
        old_key,
        &crate::model::Sample {
            at: 17,
            cpu: Some(2.0),
            memory: None,
            disk: None,
        },
    )?;
    store.save_login(&old, crate::Method::Key, "prior-key", None)?;
    store.set_auto(&old, true)?;
    let core = Workspace::open(
        dir.path().into(),
        password.clone(),
        vec![old.clone(), other.clone()],
    )
    .await?;
    let id = core.bind(&old, None).await?;
    let prior = format!("ssh://{old}");
    let binding = || Some((id.clone(), prior.clone()));
    assert!(!store.nodes()?.contains(&new));
    store.trust(&new, "next-pin")?;
    let asset = core
        .api(
            Method::POST,
            "/assets",
            json!({"node_id":id,"name":"示例","management":"process"}),
        )
        .await?;
    assert!(core.bind(&other, binding()).await.is_err());
    assert!(
        core.bind(&new, Some((id.clone(), "stale".into())))
            .await
            .is_err()
    );
    let prepared = core
        .api(
            Method::POST,
            &format!("/nodes/{id}/operations"),
            json!({"type":"command","name":"uptime","parameters":{}}),
        )
        .await?;
    assert!(core.bind(&new, binding()).await.is_err());
    core.api(
        Method::POST,
        &format!(
            "/operations/{}/cancel",
            prepared["task_id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await?;
    assert!(!store.nodes()?.contains(&new));
    assert_eq!(core.bind(&new, binding()).await?, id);
    assert!(store.nodes()?.contains(&new));
    let new_key = store.connected(&new)?;
    assert_eq!(core.bind(&new, None).await?, id);
    assert_eq!(
        core.api(
            Method::GET,
            &format!("/assets/{}", asset["id"].as_str().unwrap()),
            Value::Null
        )
        .await?["node_id"],
        id
    );
    assert_eq!(store.history(new_key)?[0].at, 17);
    assert_eq!(store.fingerprint(&old)?.as_deref(), Some("previous-pin"));
    assert_eq!(store.fingerprint(&new)?.as_deref(), Some("next-pin"));
    assert!(store.auto_nodes()?.is_empty());
    assert!(store.saved_login(&old)?.is_none() && store.saved_login(&new)?.is_none());
    core.pool.close().await;
    drop(core);
    store.connected(&old)?;
    let reopened = Workspace::open(dir.path().into(), password, store.nodes()?).await?;
    assert_ne!(reopened.bind(&old, None).await?, id);
    assert_eq!(reopened.bind(&new, None).await?, id);
    reopened.pool.close().await;
    Ok(())
}
