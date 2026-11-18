use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Runtime {
    Docker { container_id: String },
    Systemd { unit: String },
}

pub(crate) fn runtime_from_cgroups(contents: &str) -> Option<Runtime> {
    let candidates = contents.lines().filter_map(|line| {
        let (_, path) = line.split_once(':')?.1.split_once(':')?;
        if !path.starts_with('/') || path.ends_with(" (deleted)") {
            return None;
        }
        let mut previous = "";
        let mut service = None;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            let id = if previous == "docker" {
                Some(part)
            } else {
                part.strip_prefix("docker-")
                    .and_then(|name| name.strip_suffix(".scope"))
            };
            if let Some(id) = id.filter(|id| {
                id.len() == 64
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            }) {
                return Some(Runtime::Docker {
                    container_id: id.to_owned(),
                });
            }
            if service.is_none() && crate::systemd::valid_name(part) {
                service = Some(Runtime::Systemd {
                    unit: part.to_owned(),
                });
            }
            previous = part;
        }
        service
    });
    let mut docker = None;
    let mut service = None;
    let mut ambiguous_service = false;
    for candidate in candidates {
        match &candidate {
            Runtime::Docker { .. } => {
                if docker
                    .as_ref()
                    .is_some_and(|previous| previous != &candidate)
                {
                    return None;
                }
                docker = Some(candidate);
            }
            Runtime::Systemd { .. } => {
                ambiguous_service |= service
                    .as_ref()
                    .is_some_and(|previous| previous != &candidate);
                service = Some(candidate);
            }
        }
    }
    docker.or(if ambiguous_service { None } else { service })
}

#[cfg(test)]
mod tests {
    use super::{Runtime, runtime_from_cgroups};

    #[test]
    fn reads_service_and_preserves_escaped_instance_names() {
        let unit = r"worker@batch\x2djob.service";
        let contents = format!("0::/system.slice/{unit}/subgroup\n");
        assert_eq!(
            runtime_from_cgroups(&contents),
            Some(Runtime::Systemd { unit: unit.into() })
        );
    }

    #[test]
    fn reads_docker_paths_and_prefers_container_over_outer_service() {
        let id = "ab".repeat(32);
        let contents = format!(
            "3:cpu:/system.slice/docker.service\n0::/system.slice/docker-{id}.scope/inner.service\n"
        );
        let expected = Some(Runtime::Docker {
            container_id: id.clone(),
        });
        assert_eq!(runtime_from_cgroups(&contents), expected);
        assert_eq!(
            runtime_from_cgroups(&format!("7:memory:/docker/{id}\n")),
            expected
        );
    }

    #[test]
    fn omits_unknown_deleted_and_conflicting_membership() {
        for contents in [
            "0::/",
            "malformed",
            "0::/docker/01234",
            "0::/system.slice/worker.service (deleted)",
            "3:cpu:/system.slice/a.service\n2:memory:/system.slice/b.service",
        ] {
            assert_eq!(runtime_from_cgroups(contents), None, "{contents}");
        }
    }
}
