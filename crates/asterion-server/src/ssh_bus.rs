use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use asterion_protocol::{TaskProgress, TaskReply, TaskRequest, is_node_id, unix_time};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, timeout_at};

pub const MAX_PENDING: usize = 8;
const QUERY_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    Busy,
    Offline,
    Unsupported,
    Timeout,
    Rejected { code: String, message: String },
}

pub(crate) struct Dispatch {
    pub request: TaskRequest,
    pub response: oneshot::Sender<TaskReply>,
    pub progress: watch::Sender<Option<TaskProgress>>,
}

pub(crate) enum Control {
    Task(Dispatch),
    Cancel(String),
}

struct Site {
    identity: Arc<()>,
    tasks: Option<mpsc::Sender<Control>>,
}

pub struct SshBus {
    sites: Mutex<HashMap<String, Site>>,
    sample_interval_secs: u64,
    terminal_enabled: bool,
    retention_days: (u64, u64),
}

impl SshBus {
    pub fn new(sample_interval_secs: u64) -> Self {
        Self {
            sites: Mutex::new(HashMap::new()),
            sample_interval_secs,
            terminal_enabled: false,
            retention_days: (7, 90),
        }
    }

    pub fn with_terminal_enabled(mut self, enabled: bool) -> Self {
        self.terminal_enabled = enabled;
        self
    }

    pub fn with_retention_days(mut self, metric: u64, trend: u64) -> Self {
        self.retention_days = (metric, trend);
        self
    }

    pub(crate) fn retention_days(&self) -> (u64, u64) {
        self.retention_days
    }

    pub(crate) fn terminal_enabled(&self) -> bool {
        self.terminal_enabled
    }

    pub(crate) fn sample_interval_secs(&self) -> u64 {
        self.sample_interval_secs
    }

    fn sites(&self) -> MutexGuard<'_, HashMap<String, Site>> {
        self.sites.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub(crate) fn attach(self: &Arc<Self>, node_id: &str, can_monitor: bool) -> Subscription {
        let identity = Arc::new(());
        let (tx, rx) = mpsc::channel(MAX_PENDING);
        self.sites().insert(
            node_id.to_owned(),
            Site {
                identity: Arc::clone(&identity),
                tasks: can_monitor.then_some(tx),
            },
        );
        Subscription {
            bus: Arc::clone(self),
            node_id: node_id.to_owned(),
            identity,
            rx,
        }
    }

    pub async fn query(&self, node_id: &str, kind: &str, args: Value) -> Result<Value, BusError> {
        let mut entropy = [0; 16];
        getrandom::fill(&mut entropy).map_err(|_| BusError::Busy)?;
        let id: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        self.enqueue(node_id, &id, kind, args, QUERY_TIMEOUT)?
            .wait()
            .await
    }

    pub fn enqueue(
        &self,
        node_id: &str,
        request_id: &str,
        kind: &str,
        args: Value,
        deadline: Duration,
    ) -> Result<PendingTask, BusError> {
        let limit = if kind == "operation_execute" {
            930
        } else {
            120
        };
        if !is_node_id(request_id) || !(1..=limit).contains(&deadline.as_secs()) {
            return Err(BusError::Busy);
        }
        let (response, rx) = oneshot::channel();
        let (progress, updates) = watch::channel(None);
        let expires_at = unix_time()
            .ok_or(BusError::Busy)?
            .saturating_add(deadline.as_secs() as i64);
        let dispatch = Dispatch {
            request: TaskRequest {
                request_id: request_id.to_owned(),
                kind: kind.to_owned(),
                args,
                expires_at,
            },
            response,
            progress,
        };
        self.send(node_id, Control::Task(dispatch))?;
        Ok(PendingTask {
            until: Instant::now() + deadline,
            rx,
            updates,
        })
    }

    pub fn cancel(&self, node_id: &str, task_id: &str) -> Result<(), BusError> {
        self.send(node_id, Control::Cancel(task_id.to_owned()))
    }

    fn send(&self, node_id: &str, control: Control) -> Result<(), BusError> {
        let tx = {
            let sites = self.sites();
            let site = sites.get(node_id).ok_or(BusError::Offline)?;
            site.tasks.clone().ok_or(BusError::Unsupported)?
        };
        tx.try_send(control).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => BusError::Busy,
            mpsc::error::TrySendError::Closed(_) => BusError::Offline,
        })
    }
}

pub struct PendingTask {
    until: Instant,
    rx: oneshot::Receiver<TaskReply>,
    updates: watch::Receiver<Option<TaskProgress>>,
}

impl PendingTask {
    pub fn progress(&self) -> watch::Receiver<Option<TaskProgress>> {
        self.updates.clone()
    }
    pub async fn wait(self) -> Result<Value, BusError> {
        let reply = timeout_at(self.until, self.rx)
            .await
            .map_err(|_| BusError::Timeout)?
            .map_err(|_| BusError::Offline)?;
        match reply {
            TaskReply::Success { value } => Ok(value),
            TaskReply::Failure { code, .. } if code == "offline" => Err(BusError::Offline),
            TaskReply::Failure { code, .. } if code == "busy" => Err(BusError::Busy),
            TaskReply::Failure { code, .. } if code == "timeout" => Err(BusError::Timeout),
            TaskReply::Failure { code, message } => Err(BusError::Rejected { code, message }),
        }
    }
}

pub(crate) struct Subscription {
    bus: Arc<SshBus>,
    node_id: String,
    identity: Arc<()>,
    rx: mpsc::Receiver<Control>,
}

impl Subscription {
    pub(crate) async fn next(&mut self) -> Control {
        match self.rx.recv().await {
            Some(dispatch) => dispatch,
            None => std::future::pending().await,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut sites = self.bus.sites();
        if sites
            .get(&self.node_id)
            .is_some_and(|site| Arc::ptr_eq(&site.identity, &self.identity))
        {
            sites.remove(&self.node_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn dispatch_keeps_node_identity_and_disconnect_is_scoped() {
        let bus = Arc::new(SshBus::new(15));
        let legacy = bus.attach("legacy", false);
        assert_eq!(
            bus.query("legacy", "processes", json!({})).await,
            Err(BusError::Unsupported)
        );
        let stale = bus.attach("a", true);
        let mut current = bus.attach("a", true);
        drop(stale);

        let response = async {
            let Control::Task(request) = current.next().await else {
                panic!("expected a task dispatch");
            };
            assert_eq!(request.request.kind, "processes");
            request
                .response
                .send(TaskReply::Success {
                    value: json!(["a"]),
                })
                .unwrap();
        };
        let (result, ()) = tokio::join!(bus.query("a", "processes", json!({})), response);
        assert_eq!(result.unwrap(), json!(["a"]));
        assert_eq!(
            bus.query("other", "processes", json!({})).await,
            Err(BusError::Offline)
        );
        drop(legacy);
    }
}
