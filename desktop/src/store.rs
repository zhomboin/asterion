use crate::model::{HISTORY, Node, Sample};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf};

pub struct Store(Connection);

impl Store {
    pub(super) fn db(&self) -> &Connection {
        &self.0
    }
    pub fn workspace_configured() -> Result<bool> {
        let path = Self::directory()?.join("workspace.sqlite3");
        if !path.try_exists()? {
            return Ok(false);
        }
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let table: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='panel_admin' AND type='table')",
            [],
            |r| r.get(0),
        )?;
        Ok(table
            && db.query_row(
                "SELECT EXISTS(SELECT 1 FROM panel_admin WHERE id=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?)
    }
    pub fn directory() -> Result<PathBuf> {
        let dir = std::env::var_os("ASTERION_DESKTOP_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Asterion")))
            .context("找不到用户数据目录")?;
        fs::create_dir_all(&dir).context("无法创建桌面数据目录")?;
        Ok(dir)
    }
    pub fn open() -> Result<Self> {
        Self::open_in(&Self::directory()?)
    }
    pub fn open_in(dir: &std::path::Path) -> Result<Self> {
        Self::init(Connection::open(dir.join("desktop.sqlite3")).context("无法打开桌面数据库")?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS hosts(host TEXT,port INTEGER,fingerprint TEXT NOT NULL,PRIMARY KEY(host,port));
            CREATE TABLE IF NOT EXISTS nodes(id INTEGER PRIMARY KEY,host TEXT NOT NULL,port INTEGER NOT NULL,user TEXT NOT NULL,UNIQUE(host,port,user));
            CREATE TABLE IF NOT EXISTS auto_nodes(address TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS saved_logins(address TEXT PRIMARY KEY,method TEXT NOT NULL,source TEXT NOT NULL,secret BLOB);
            CREATE TABLE IF NOT EXISTS samples(node INTEGER REFERENCES nodes(id),at INTEGER,cpu REAL,memory REAL,disk REAL,PRIMARY KEY(node,at));")?;
        Ok(Self(db))
    }

    pub fn nodes(&self) -> Result<Vec<Node>> {
        let mut q = self
            .0
            .prepare("SELECT host,port,user FROM nodes ORDER BY id DESC")?;
        Ok(q.query_map([], |r| {
            Ok(Node {
                host: r.get(0)?,
                port: r.get(1)?,
                user: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    pub fn move_shortcut(&mut self, old: &str, new: &str) -> Result<()> {
        if old == new {
            return Ok(());
        }
        let nodes = self.nodes()?;
        let find = |address: &str| nodes.iter().find(|n| format!("ssh://{n}") == address);
        let Some(prior) = find(old) else {
            return Ok(());
        };
        let target = Node::from_address(new)?;
        let tx = self.0.transaction()?;
        let id = |n: &Node| {
            tx.query_row(
                "SELECT id FROM nodes WHERE host=? AND port=? AND user=?",
                params![n.host, n.port, n.user],
                |r| r.get::<_, i64>(0),
            )
        };
        tx.execute(
            "INSERT OR IGNORE INTO nodes(host,port,user) VALUES(?,?,?)",
            params![target.host, target.port, target.user],
        )?;
        let (a, b) = (id(prior)?, id(&target)?);
        tx.execute("INSERT OR IGNORE INTO samples(node,at,cpu,memory,disk) SELECT ?,at,cpu,memory,disk FROM samples WHERE node=?",params![b,a])?;
        tx.execute("DELETE FROM samples WHERE node=?1 OR (node=?2 AND at < (SELECT at FROM samples WHERE node=?2 ORDER BY at DESC LIMIT 1 OFFSET ?3))",params![a,b,HISTORY-1])?;
        tx.execute("DELETE FROM nodes WHERE id=?", [a])?;
        tx.execute(
            "DELETE FROM auto_nodes WHERE address=?",
            [prior.to_string()],
        )?;
        tx.execute(
            "DELETE FROM saved_logins WHERE address=?",
            [prior.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn forget(&mut self, address: &str) -> Result<()> {
        let Some(node) = self
            .nodes()?
            .into_iter()
            .find(|n| format!("ssh://{n}") == address)
        else {
            return Ok(());
        };
        let tx = self.0.transaction()?;
        let id: Option<i64> = tx
            .query_row(
                "SELECT id FROM nodes WHERE host=? AND port=? AND user=?",
                params![node.host, node.port, node.user],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            tx.execute("DELETE FROM auto_nodes WHERE address=?", [node.to_string()])?;
            tx.execute(
                "DELETE FROM saved_logins WHERE address=?",
                [node.to_string()],
            )?;
            tx.execute("DELETE FROM samples WHERE node=?", [id])?;
            tx.execute("DELETE FROM nodes WHERE id=?", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn fingerprint(&self, n: &Node) -> Result<Option<String>> {
        Ok(self
            .0
            .query_row(
                "SELECT fingerprint FROM hosts WHERE host=? AND port=?",
                params![n.host, n.port],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn trust(&self, n: &Node, fingerprint: &str) -> Result<()> {
        self.0.execute(
            "INSERT INTO hosts VALUES(?,?,?) ON CONFLICT DO NOTHING",
            params![n.host, n.port, fingerprint],
        )?;
        ensure!(
            self.fingerprint(n)?.as_deref() == Some(fingerprint),
            "已保存的主机指纹不同，连接已阻止"
        );
        Ok(())
    }

    pub fn connected(&self, n: &Node) -> Result<i64> {
        self.0.execute(
            "INSERT INTO nodes(host,port,user) VALUES(?,?,?) ON CONFLICT DO NOTHING",
            params![n.host, n.port, n.user],
        )?;
        Ok(self.0.query_row(
            "SELECT id FROM nodes WHERE host=? AND port=? AND user=?",
            params![n.host, n.port, n.user],
            |r| r.get(0),
        )?)
    }

    pub fn save(&mut self, id: i64, s: &Sample) -> Result<()> {
        let tx = self.0.transaction()?;
        tx.execute("INSERT INTO samples VALUES(?,?,?,?,?) ON CONFLICT(node,at) DO UPDATE SET cpu=excluded.cpu,memory=excluded.memory,disk=excluded.disk", params![id,s.at,s.cpu,s.memory,s.disk])?;
        tx.execute("DELETE FROM samples WHERE node=?1 AND at < (SELECT at FROM samples WHERE node=?1 ORDER BY at DESC LIMIT 1 OFFSET ?2)",params![id,(HISTORY-1) as i64])?;
        tx.commit()?;
        Ok(())
    }

    pub fn history(&self, id: i64) -> Result<Vec<Sample>> {
        let mut q = self
            .0
            .prepare("SELECT at,cpu,memory,disk FROM samples WHERE node=? ORDER BY at")?;
        Ok(q.query_map([id], |r| {
            Ok(Sample {
                at: r.get(0)?,
                cpu: r.get(1)?,
                memory: r.get(2)?,
                disk: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_are_immutable_and_history_is_bounded_per_node() {
        let mut store = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        let n = Node::parse("host.invalid", "2222", "user").unwrap();
        store.trust(&n, "first").unwrap();
        assert!(store.trust(&n, "changed").is_err());
        assert_eq!(store.fingerprint(&n).unwrap().as_deref(), Some("first"));
        let id = store.connected(&n).unwrap();
        for at in 0..=HISTORY as i64 {
            store
                .save(
                    id,
                    &Sample {
                        at,
                        cpu: None,
                        memory: Some(0.0),
                        disk: None,
                    },
                )
                .unwrap();
        }
        let history = store.history(id).unwrap();
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history[0].at, 1);
        assert_eq!(store.nodes().unwrap(), vec![n]);
    }
}
