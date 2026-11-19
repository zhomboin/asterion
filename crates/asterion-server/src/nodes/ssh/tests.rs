use super::*;

#[test]
fn confirmation_binds_owner_fingerprint_and_expiry() {
    let mut prepared = Prepared {
        owner: [4; 32],
        reinstall: false,
        at: Instant::now(),
        target: Target {
            host: "node.example.invalid".into(),
            user: "operator".into(),
            port: 22,
            node_id: None,
        },
        plan: Plan {
            address: "192.0.2.1".parse().unwrap(),
            center: String::new(),
            commands: Vec::new(),
            fingerprint: "verified".into(),
            manifest: String::new(),
        },
    };
    assert!(prepared.confirms(&[4; 32], "verified"));
    assert!(!prepared.confirms(&[8; 32], "verified"));
    assert!(!prepared.confirms(&[4; 32], "different"));
    prepared.at -= Duration::from_secs(121);
    assert!(!prepared.confirms(&[4; 32], "verified"));
}

#[test]
fn restricts_target_syntax_and_ports() {
    let mut target = Target {
        host: "2001:db8::1".into(),
        user: "agent-install".into(),
        port: 2222,
        node_id: None,
    };
    assert!(target.valid());
    for invalid in [
        "",
        "-flags",
        "ssh://example.invalid",
        "x@y",
        "x\nignored",
        "server..name",
    ] {
        target.host = invalid.into();
        assert!(!target.valid());
    }
    target.host = "server.example.invalid".into();
    target.port = 0;
    assert!(!target.valid());
}
#[tokio::test]
async fn deployment_distinguishes_pending_enrolled_and_revoked_nodes() {
    let pool = crate::database::connect("sqlite::memory:").await.unwrap();
    let id = "ab".repeat(16);
    let mut tx = pool.begin().await.unwrap();
    assert!(enrolled(&mut tx, &id).await.is_err());
    sqlx::query("INSERT INTO nodes (id,name,kind) VALUES (?,'fixture','linux')")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(!enrolled(&mut tx, &id).await.unwrap());
    sqlx::query("INSERT INTO agent_credentials (node_id,fingerprint,created_at,connection_generation) VALUES (?,zeroblob(32),unixepoch(),1)")
        .bind(&id).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE nodes SET last_seen_at=unixepoch() WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(enrolled(&mut tx, &id).await.unwrap());
    credentials::revoke(&mut tx, &id).await.unwrap();
    assert!(enrolled(&mut tx, &id).await.is_err());
}
