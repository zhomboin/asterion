use super::*;
use russh::keys::{Algorithm, PrivateKey};
use russh::{
    ChannelId, Pty,
    server::{self, Auth, ChannelOpenHandle, Msg},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Fixture {
    password: String,
    auth_calls: Arc<AtomicUsize>,
    root: Arc<AtomicBool>,
}

impl server::Handler for Fixture {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        Ok(if password == self.password {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        id: ChannelId,
        command: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.channel_success(id)?;
        let output = if command == b"id -u" {
            if self.root.load(Ordering::SeqCst) {
                "0".into()
            } else {
                "1000".into()
            }
        } else if command == include_bytes!("../../../deploy/ssh/sample-basic.sh") {
            let capacity =
                serde_json::json!({"total_bytes":100,"available_bytes":40,"used_bytes":60});
            serde_json::json!({"outcome":"success","value":{"sample":{"captured_at":1000,"logical_cores":2,"cpu_percent":15.5,"load":[0,1,0],"memory":capacity,"swap":capacity,"interfaces":[],"filesystems":[{"mountpoint":"/","fs_type":"test","total_bytes":100,"available_bytes":40}]}}}).to_string()
        } else {
            s.exit_status_request(id, 1)?;
            s.close(id)?;
            return Ok(());
        };
        s.data(id, output.into_bytes())?;
        s.exit_status_request(id, 0)?;
        s.eof(id)?;
        s.close(id)?;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.channel_success(id)
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.channel_success(id)
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.data(id, data.to_vec())
    }
    async fn window_change_request(
        &mut self,
        id: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.data(id, format!("{cols}x{rows}").into_bytes())
    }
}

#[tokio::test]
async fn verified_connection_supports_metrics_and_interactive_channels() -> Result<()> {
    timeout(Duration::from_secs(25), exercise()).await??;
    Ok(())
}

async fn exercise() -> Result<()> {
    let host = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    let pin = host.public_key().fingerprint(HashAlg::Sha256).to_string();
    let password = rand::random::<u64>().to_string();
    let fixture = Fixture {
        password: password.clone(),
        auth_calls: Default::default(),
        root: Default::default(),
    };
    let server = fixture.clone();
    let config = Arc::new(server::Config {
        keys: vec![host],
        auth_rejection_time: Duration::from_millis(1),
        ..Default::default()
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let node = Node::parse(
        &listener.local_addr()?.ip().to_string(),
        &listener.local_addr()?.port().to_string(),
        "test",
    )?;
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = config.clone();
            let server = server.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, server).await {
                    let _ = session.await;
                }
            });
        }
    });
    let auth = || Credentials::Password(Zeroizing::new(password.clone()));
    assert!(matches!(login(node.clone(),None,auth()).await?,Login::Verify(fp) if fp==pin));
    assert!(
        login(node.clone(), Some("untrusted".into()), auth())
            .await
            .is_err()
    );
    assert_eq!(fixture.auth_calls.load(Ordering::SeqCst), 0);
    assert!(
        login(
            node.clone(),
            Some(pin.clone()),
            Credentials::Password(Default::default())
        )
        .await
        .is_err()
    );
    let Login::Connected(session) = login(node, Some(pin), auth()).await? else {
        bail!("未建立连接")
    };
    let s = sample(session.clone()).await?;
    assert_eq!(
        (s.cpu, s.memory, s.disk),
        (Some(15.5), Some(60.), Some(60.))
    );
    fixture.root.store(true, Ordering::SeqCst);
    assert!(terminal(&session, 80, 24).await.is_err());
    fixture.root.store(false, Ordering::SeqCst);
    let mut shell = terminal(&session, 80, 24).await?;
    shell.data("节点".as_bytes()).await?;
    assert!(
        matches!(shell.wait().await,Some(ChannelMsg::Data{data}) if data.as_ref()=="节点".as_bytes())
    );
    shell.window_change(100, 30, 0, 0).await?;
    assert!(matches!(shell.wait().await,Some(ChannelMsg::Data{data}) if data.as_ref()==b"100x30"));
    shell.close().await?;
    session
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await?;
    task.abort();
    let _ = task.await;
    Ok(())
}
