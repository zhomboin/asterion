use crate::model::{HISTORY, Node, Sample};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf};

pub struct Store(Connection);

impl Store {
    pub fn open() -> Result<Self> {
        let dir = std::env::var_os("ASTERION_DESKTOP_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Asterion")))
            .context("找不到用户数据目录")?;
        fs::create_dir_all(&dir).context("无法创建桌面数据目录")?;
        Self::init(Connection::open(dir.join("desktop.sqlite3")).context("无法打开桌面数据库")?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS hosts(host TEXT,port INTEGER,fingerprint TEXT NOT NULL,PRIMARY KEY(host,port));
            CREATE TABLE IF NOT EXISTS nodes(id INTEGER PRIMARY KEY,host TEXT NOT NULL,port INTEGER NOT NULL,user TEXT NOT NULL,UNIQUE(host,port,user));
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
