use asterion_server::database;

#[tokio::test]
async fn migration_keeps_assets_and_runtimes_scoped_to_nodes() {
    let pool = database::connect("sqlite::memory:").await.unwrap();

    sqlx::query("INSERT INTO hosts (id, name, kind) VALUES ('host-a', '主机', 'windows')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO nodes (id, host_id, name, kind) VALUES
         ('node-a', 'host-a', '节点', 'wsl'),
         ('node-b', NULL, '节点', 'linux')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO runtimes (id, node_id, kind, identity) VALUES
         ('runtime-a', 'node-a', 'docker', 'engine-a'),
         ('runtime-b', 'node-b', 'docker', 'engine-a')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO service_assets (id, node_id, name, management) VALUES
         ('asset-a', 'node-a', '同名服务', 'docker'),
         ('asset-b', 'node-b', '同名服务', 'docker')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let duplicate_runtime = sqlx::query(
        "INSERT INTO runtimes (id, node_id, kind, identity)
         VALUES ('runtime-c', 'node-a', 'docker', 'engine-a')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        duplicate_runtime
            .as_database_error()
            .unwrap()
            .is_unique_violation()
    );

    let unknown_node = sqlx::query(
        "INSERT INTO service_assets (id, node_id, name, management)
         VALUES ('asset-c', 'missing-node', '孤立服务', 'other')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        unknown_node
            .as_database_error()
            .unwrap()
            .is_foreign_key_violation()
    );

    sqlx::query("DELETE FROM nodes WHERE id = 'node-a'")
        .execute(&pool)
        .await
        .expect_err("删除节点时应保留已登记的资产");
    let remaining_runtimes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runtimes WHERE node_id = 'node-a'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_runtimes, 1);
}
