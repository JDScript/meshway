//! Functional carrier-only fixture. No Meshway identities or production payloads.
//! Used by scripts/xray_interop.py; credentials arrive via private environment.
use std::{env, error::Error, io, net::IpAddr, time::Duration};

use meshway_transport_socks5::{
    Auth, ClientConfig, Credentials, ServerConfig, Target, client_stream, server_stream,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const GREETING: &[u8] = b"meshway-socks5-acceptor-first\0\xff";

fn auth(mode: &str) -> Result<Auth, Box<dyn Error>> {
    match mode {
        "noauth" => Ok(Auth::NoAuth),
        "password" => {
            let username = env::var("MESHWAY_SOCKS5_TEST_USERNAME")?;
            let password = env::var("MESHWAY_SOCKS5_TEST_PASSWORD")?;
            Ok(Auth::UsernamePassword(Credentials::new(
                username.as_bytes(),
                password.as_bytes(),
            )?))
        }
        _ => Err(io::Error::other("expected noauth or password").into()),
    }
}

fn target(host: &str, port: &str) -> Result<Target, Box<dyn Error>> {
    let port = port.parse()?;
    Ok(match host.parse::<IpAddr>() {
        Ok(ip) => Target::ip(ip, port)?,
        Err(_) => Target::domain(host, port)?,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("client") if args.len() == 6 => {
            let config = ClientConfig { auth: auth(&args[5])?, target: target(&args[3], &args[4])? };
            tokio::time::timeout(Duration::from_secs(8), async {
                let socket = TcpStream::connect(&args[2]).await?;
                let (mut stream, _) = client_stream(socket, &config).await?;
                let mut greeting = vec![0; GREETING.len()];
                stream.read_exact(&mut greeting).await?;
                if greeting != GREETING { return Err(io::Error::other("incorrect acceptor-first payload").into()); }
                let payload: Vec<u8> = (0..65536).map(|i| i as u8).collect();
                stream.write_all(&payload).await?;
                let mut reply = vec![0; payload.len()];
                stream.read_exact(&mut reply).await?;
                if payload != reply { return Err(io::Error::other("incorrect binary echo").into()); }
                stream.shutdown().await?;
                Ok::<(), Box<dyn Error>>(())
            }).await??;
            println!("PASS our client: acceptor-first and 65536-byte binary echo");
        }
        Some("server") if args.len() == 5 => {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let config = ServerConfig { auth: auth(&args[4])?, expected_target: target(&args[2], &args[3])?, bound_address: listener.local_addr()? };
            println!("PORT {}", listener.local_addr()?.port());
            loop {
                let (socket, _) = listener.accept().await?;
                let config = config.clone();
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(10), async {
                        let (mut stream, _) = server_stream(socket, &config).await?;
                        stream.write_all(GREETING).await?;
                        let mut buffer = [0; 8192];
                        loop {
                            let count = stream.read(&mut buffer).await?;
                            if count == 0 { stream.shutdown().await?; break; }
                            stream.write_all(&buffer[..count]).await?;
                        }
                        Ok::<(), meshway_transport_socks5::Error>(())
                    }).await;
                });
            }
        }
        _ => return Err(io::Error::other("usage: interop_probe client PROXY HOST PORT noauth|password; or server HOST PORT noauth|password").into()),
    }
    Ok(())
}
