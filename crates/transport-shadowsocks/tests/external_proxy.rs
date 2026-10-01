//! Functional probes only. The helper supplies fresh isolated credentials and
//! pinned sing-box; these tests do not perform Meshway Noise or measure throughput.

use std::{env, error::Error, time::Duration};

use meshway_transport_shadowsocks::{
    Cipher, ClientConfig, SecretPassword, ServerConfig, Target, client_stream, server_stream,
};
use shadowsocks::relay::socks5::{
    Address, Command, HandshakeRequest, HandshakeResponse, Reply, TcpRequestHeader,
    TcpResponseHeader,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

fn credential() -> Result<(Cipher, SecretPassword), Box<dyn Error + Send + Sync>> {
    Ok((
        env::var("MESHWAY_SS_CIPHER")?.parse()?,
        SecretPassword::new(env::var("MESHWAY_SS_PASSWORD")?)?,
    ))
}

fn payload() -> Vec<u8> {
    (0..40000).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
#[ignore = "run with tests/run_sing_box_interop.py"]
async fn meshway_client_to_sing_box_server() -> TestResult {
    tokio::time::timeout(Duration::from_secs(15), async {
        let listener = TcpListener::bind(env::var("MESHWAY_SS_ENDPOINT_BIND")?).await?;
        let (cipher, password) = credential()?;
        let config = ClientConfig::new(
            cipher,
            password,
            Target::domain("host.docker.internal", listener.local_addr()?.port())?,
        )?;
        let request = payload();
        let response: Vec<_> = request.iter().copied().rev().collect();
        let endpoint = async {
            let (mut stream, _) = listener.accept().await?;
            let mut received = vec![0; request.len()];
            stream.read_exact(&mut received).await?;
            assert_eq!(received, request);
            // The real destination deliberately responds only after client data.
            stream.write_all(&response).await?;
            let mut ack = [0; 4];
            stream.read_exact(&mut ack).await?;
            assert_eq!(&ack, b"\0ok\xff");
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        };
        let client = async {
            let tcp = TcpStream::connect(env::var("MESHWAY_SS_PROXY")?).await?;
            let mut stream = client_stream(tcp, &config).await?;
            stream.write_all(&request).await?;
            let mut received = vec![0; response.len()];
            stream.read_exact(&mut received).await?;
            assert_eq!(received, response);
            stream.write_all(b"\0ok\xff").await?;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        };
        tokio::try_join!(endpoint, client)?;
        Ok(())
    })
    .await?
}

#[tokio::test]
#[ignore = "run with tests/run_sing_box_interop.py"]
async fn sing_box_client_to_meshway_server() -> TestResult {
    tokio::time::timeout(Duration::from_secs(15), async {
        let listener = TcpListener::bind(env::var("MESHWAY_SS_ADAPTER_BIND")?).await?;
        let (cipher, password) = credential()?;
        let config =
            ServerConfig::new(cipher, password, Target::domain("meshway.internal", 19001)?)?;
        let request = payload();
        let response: Vec<_> = request.iter().copied().rev().collect();
        let server = async {
            let (tcp, _) = listener.accept().await?;
            let mut stream = server_stream(tcp, &config).await?;
            let mut received = vec![0; request.len()];
            stream.read_exact(&mut received).await?;
            assert_eq!(received, request);
            stream.write_all(&response).await?;
            let mut ack = [0; 4];
            stream.read_exact(&mut ack).await?;
            assert_eq!(&ack, b"\0ok\xff");
            // Reverse business writes after the client-first handshake exchange.
            stream.write_all(b"server-push\0\xff").await?;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        };
        let client = async {
            let mut stream = TcpStream::connect(env::var("MESHWAY_SS_SOCKS")?).await?;
            HandshakeRequest::new(vec![0]).write_to(&mut stream).await?;
            assert_eq!(
                HandshakeResponse::read_from(&mut stream)
                    .await?
                    .chosen_method,
                0
            );
            TcpRequestHeader::new(
                Command::TcpConnect,
                Address::DomainNameAddress("meshway.internal".into(), 19001),
            )
            .write_to(&mut stream)
            .await?;
            assert!(matches!(
                TcpResponseHeader::read_from(&mut stream).await?.reply,
                Reply::Succeeded
            ));
            stream.write_all(&request).await?;
            let mut received = vec![0; response.len()];
            stream.read_exact(&mut received).await?;
            assert_eq!(received, response);
            stream.write_all(b"\0ok\xff").await?;
            let mut push = [0; b"server-push\0\xff".len()];
            stream.read_exact(&mut push).await?;
            assert_eq!(&push, b"server-push\0\xff");
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        };
        tokio::try_join!(server, client)?;
        Ok(())
    })
    .await?
}
