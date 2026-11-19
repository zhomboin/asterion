use super::*;

#[test]
fn confirmation_binds_owner_fingerprint_and_expiry() {
    let mut prepared = Prepared {
        owner: [4; 32],
        at: Instant::now(),
        target: Target {
            host: "node.example.invalid".into(),
            user: "operator".into(),
            port: 22,
        },
        plan: Plan {
            address: "192.0.2.1".parse().unwrap(),
            center: String::new(),
            commands: Vec::new(),
            fingerprint: "verified".into(),
            manifest: String::new(),
        },
    };
    assert!(prepared.confirms(&[4; 32], "verified"));
    assert!(!prepared.confirms(&[8; 32], "verified"));
    assert!(!prepared.confirms(&[4; 32], "different"));
    prepared.at -= Duration::from_secs(121);
    assert!(!prepared.confirms(&[4; 32], "verified"));
}

#[test]
fn restricts_target_syntax_and_ports() {
    let mut target = Target {
        host: "2001:db8::1".into(),
        user: "agent-install".into(),
        port: 2222,
    };
    assert!(target.valid());
    for invalid in [
        "",
        "-flags",
        "ssh://example.invalid",
        "x@y",
        "x\nignored",
        "server..name",
    ] {
        target.host = invalid.into();
        assert!(!target.valid());
    }
    target.host = "server.example.invalid".into();
    target.port = 0;
    assert!(!target.valid());
}
