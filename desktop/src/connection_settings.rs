use crate::{Method, model::Node, store::Store, vault};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

pub struct Saved {
    pub method: Method,
    pub source: String,
    pub has_secret: bool,
    pub auto: bool,
}
impl Store {
    pub fn saved_login(&self, node: &Node) -> Result<Option<Saved>> {
        let row = self
            .db()
            .query_row(
                "SELECT method,source,secret IS NOT NULL,
             EXISTS(SELECT 1 FROM auto_nodes a WHERE a.address=saved_logins.address)
             FROM saved_logins WHERE address=?",
                [node.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(method, source, has_secret, auto)| {
            Ok(Saved {
                method: match method.as_str() {
                    "password" => Method::Password,
                    "key" => Method::Key,
                    "config" => Method::Config,
                    _ => anyhow::bail!("未知的认证方式"),
                },
                source,
                has_secret,
                auto,
            })
        })
        .transpose()
    }
    pub fn password(&self, node: &Node) -> Result<Zeroizing<String>> {
        let key = node.to_string();
        let sealed: Vec<u8> = self
            .db()
            .query_row(
                "SELECT secret FROM saved_logins WHERE address=? AND method='password'",
                [&key],
                |r| r.get(0),
            )
            .context("此地址没有保存的凭据")?;
        let plain = vault::protect(&sealed, &key, false)?;
        Ok(Zeroizing::new(std::str::from_utf8(&plain)?.to_owned()))
    }
    pub fn save_login(
        &self,
        node: &Node,
        method: Method,
        source: &str,
        password: Option<&str>,
    ) -> Result<()> {
        let address = node.to_string();
        let secret = password
            .map(|s| {
                ensure!(
                    method == Method::Password && !s.is_empty(),
                    "保存的密码不能为空"
                );
                vault::protect(s.as_bytes(), &address, true)
            })
            .transpose()?;
        let name = match method {
            Method::Password => "password",
            Method::Key => "key",
            Method::Config => "config",
        };
        self.db().execute(
            "INSERT INTO saved_logins(address,method,source,secret) VALUES(?,?,?,?)
             ON CONFLICT(address) DO UPDATE SET method=excluded.method,
             source=excluded.source,secret=excluded.secret",
            params![address, name, source, secret.as_ref().map(|s| s.as_slice())],
        )?;
        Ok(())
    }
    pub fn auto_nodes(&self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        for n in self.nodes()? {
            if self.saved_login(&n)?.is_some_and(|s| s.auto) {
                nodes.push(n);
            }
        }
        Ok(nodes)
    }
    pub fn set_auto(&self, n: &Node, enable: bool) -> Result<()> {
        if enable {
            self.db().execute(
                "INSERT OR IGNORE INTO auto_nodes SELECT address FROM saved_logins
                 WHERE address=? AND (method!='password' OR secret IS NOT NULL)",
                [n.to_string()],
            )?;
        } else {
            self.db()
                .execute("DELETE FROM auto_nodes WHERE address=?", [n.to_string()])?;
        }
        Ok(())
    }
    pub fn forget_login(&self, node: &Node) -> Result<()> {
        self.set_auto(node, false)?;
        self.db().execute(
            "DELETE FROM saved_logins WHERE address=?",
            [node.to_string()],
        )?;
        Ok(())
    }
}
