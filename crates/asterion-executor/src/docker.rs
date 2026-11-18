use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use asterion_protocol::operation::{OperationAction, ServiceVerb};
use serde::{Deserialize, Serialize};

use crate::{ExecError, gateway, policy, run, time_budget, verb_name};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorPolicy {
    engine_socket: PathBuf,
    targets: Vec<policy::DockerTarget>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    engine_id: String,
    container_id: String,
    verb: ServiceVerb,
    expires_at: i64,
}

pub(crate) async fn execute(
    address: &Path,
    engine_id: &str,
    container_id: &str,
    verb: ServiceVerb,
    expires_at: i64,
    deadline: Instant,
) -> Result<run::ResultData, ExecError> {
    gateway::execute(
        address,
        &Request {
            engine_id: engine_id.to_owned(),
            container_id: container_id.to_owned(),
            verb,
            expires_at,
        },
        deadline,
    )
    .await
}

pub async fn serve(address: &Path, file: &Path) -> Result<(), ExecError> {
    let policy: OperatorPolicy = policy::read_policy(file)?;
    policy::validate_targets(&policy.targets)?;
    policy::validate_socket_path(&policy.engine_socket)?;
    if address == policy.engine_socket
        || !policy
            .engine_socket
            .to_str()
            .is_some_and(|path| !path.contains([':', '#', '?', '[', ']', '%']))
    {
        return Err(ExecError::POLICY);
    }

    let policy = Arc::new(policy);
    gateway::serve(address, move |request: Request| {
        let policy = Arc::clone(&policy);
        async move { apply(&policy, request).await }
    })
    .await
}

async fn apply(policy: &OperatorPolicy, request: Request) -> Result<run::ResultData, ExecError> {
    let deadline = Instant::now() + time_budget(request.expires_at)?.min(Duration::from_secs(35));
    let action = OperationAction::Docker {
        engine_id: request.engine_id.clone(),
        container_id: request.container_id.clone(),
        verb: request.verb,
    };
    if !action.valid()
        || !policy::allowed_container(&policy.targets, &request.engine_id, &request.container_id)
    {
        return Err(ExecError::DENIED);
    }

    let docker = [
        Path::new("/usr/bin/docker"),
        Path::new("/usr/local/bin/docker"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or(ExecError::FAILED)?;
    let common = [
        "--config".to_owned(),
        "/var/empty".to_owned(),
        "--host".to_owned(),
        format!("unix://{}", policy.engine_socket.display()),
    ];
    let mut args = common.to_vec();
    args.extend(["info".into(), "--format".into(), "{{.ID}}".into()]);
    let info = run::command(
        docker,
        &args,
        deadline.min(Instant::now() + Duration::from_secs(5)),
    )
    .await?;
    if info.exit_code != Some(0) || info.truncated || info.output.trim() != request.engine_id {
        return Err(ExecError::new(
            "target_changed",
            "当前 Docker Engine 与授权目标不一致",
        ));
    }
    let mut args = common.to_vec();
    args.extend([
        "container".into(),
        verb_name(request.verb).into(),
        "--".into(),
        request.container_id,
    ]);
    run::command(docker, &args, deadline).await
}
