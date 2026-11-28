use super::{
    Manager,
    pages::{Kind, string},
    view::items,
};
use serde_json::Value;
impl Manager {
    pub(super) fn local_rows(&self) -> Option<Vec<&Value>> {
        let field = match self.page.kind {
            Kind::Accounts => "users",
            Kind::Docker => "containers",
            Kind::Units => "units",
            Kind::Monitors => "monitors",
            Kind::MonitorHistory => "history",
            _ => return None,
        };
        let q = self.search.trim().to_lowercase();
        Some(
            items(&self.value, field)
                .iter()
                .filter(|r| match self.page.kind {
                    Kind::Accounts => {
                        string(r, "username").to_lowercase().contains(&q) || string(r, "uid") == q
                    }
                    Kind::Monitors => self.page.item["disabled"] == "yes" || r["enabled"] == true,
                    _ => true,
                })
                .collect(),
        )
    }
    pub(super) fn visible_rows(&self) -> Vec<&Value> {
        self.local_rows()
            .unwrap_or_default()
            .into_iter()
            .skip(self.offset)
            .take(24)
            .collect()
    }
}
