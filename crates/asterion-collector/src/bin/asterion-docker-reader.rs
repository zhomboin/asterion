use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = std::env::var_os("ASTERION_DOCKER_READER_SOCKET")
        .map(PathBuf::from)
        .ok_or("未配置 ASTERION_DOCKER_READER_SOCKET")?;
    let engine = std::env::var_os("ASTERION_DOCKER_ENGINE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/run/docker.sock"));
    asterion_collector::serve_docker_reader(&listener, &engine)
}
