use super::*;
impl Workspace {
    pub(super) async fn bind(
        &self,
        node: &Node,
        rebind: Option<(String, String)>,
    ) -> Result<String> {
        let address = format!("ssh://{node}");
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query_scalar::<_, String>("SELECT id FROM nodes WHERE ssh_profile=?")
            .bind(&address)
            .fetch_optional(&mut *tx)
            .await?;
        let id = if let Some((id, old)) = rebind {
            ensure!(
                existing.as_ref().is_none_or(|other| other == &id),
                "目标 SSH 地址已登记在其他节点"
            );
            let pending = sqlx::query_scalar::<_,bool>(
                "SELECT EXISTS(SELECT 1 FROM operation_jobs WHERE node_id=?
                 AND (state IN ('queued','running') OR (state='prepared' AND expires_at>unixepoch())))",
            )
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(!pending, "此节点仍有待确认或执行中的任务");
            let rows = sqlx::query(
                "UPDATE nodes SET ssh_profile=?, ssh_online_until=NULL, ssh_error=NULL,
                 updated_at=unixepoch() WHERE id=? AND ssh_profile=?",
            )
            .bind(&address)
            .bind(&id)
            .bind(&old)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            ensure!(rows == 1, "节点连接资料已变更，请重新读取");
            if old != address {
                sqlx::query("INSERT INTO desktop_rebindings(old,new) VALUES(?,?)")
                    .bind(&old)
                    .bind(&address)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                "INSERT INTO node_audit_events(actor,node_id,action) VALUES('local',?,'node_update')",
            )
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            }
            id
        } else if let Some(id) = existing {
            id
        } else {
            // 地址可以重新分配，已有节点的历史记录始终由原 ID 标识。
            let id = sqlx::query_scalar::<_, String>(
                "SELECT CASE WHEN EXISTS(SELECT 1 FROM nodes WHERE id=?1)
                 THEN lower(hex(randomblob(16))) ELSE ?1 END",
            )
            .bind(Self::id(node))
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO nodes(id,name,kind,ssh_profile) VALUES(?,?,'linux',?)")
                .bind(&id)
                .bind(node.to_string())
                .bind(&address)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO node_audit_events(actor,node_id,action) VALUES('local',?,'node_create')",
            )
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            id
        };
        tx.commit().await?;
        reconcile(&self.pool, &self.directory).await?;
        Ok(id)
    }
}
pub async fn reconcile(pool: &SqlitePool, dir: &std::path::Path) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT old,new FROM desktop_rebindings ORDER BY rowid",
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut store = Store::open_in(dir)?;
    for (old, new) in rows {
        store.move_shortcut(&old, &new)?;
        sqlx::query("DELETE FROM desktop_rebindings WHERE old=? AND new=?")
            .bind(old)
            .bind(new)
            .execute(pool)
            .await?;
    }
    Ok(())
}
