use crate::{CarrierMode, Cli, Result};
use meshway_model::VlessStream;
use meshway_transport::spec::{
    Endpoint, Listener, ListenerTls, SecretString, TlsMode, TlsVersion, TrojanListener,
    VlessListener,
};

pub(super) async fn listener(cli: &Cli, host: String) -> Result<Listener> {
    // Independent material from the controller's HTTPS key. Only R receives the
    // private key through its authenticated execution configuration; dialers
    // receive the exact leaf certificate through the normal compiler/TLS path.
    let tls = ListenerTls {
        mode: TlsMode::Provided,
        server_name: meshway_packet_fixture::CONTROLLER_NAME.into(),
        certificate: Some(SecretString::new(
            tokio::fs::read_to_string(cli.public.join("carrier/server.pem")).await?,
        )),
        private_key: Some(SecretString::new(
            tokio::fs::read_to_string(cli.private.join("carrier/server.key")).await?,
        )),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: vec![],
        handshake_timeout: "5s".parse()?,
    };
    let endpoints = vec![Endpoint {
        host,
        port: 35000.try_into()?,
    }];
    Ok(match cli.transport {
        CarrierMode::Vless => Listener::Vless(VlessListener {
            enabled: true,
            remark: String::new(),
            listen: "0.0.0.0".parse()?,
            listen_port: 35000.try_into()?,
            endpoints,
            bind_interface: String::new(),
            socket: Default::default(),
            tcp: Default::default(),
            stream: VlessStream::Tcp,
            uuid: None,
            flow: String::new(),
            tls,
            websocket: None,
        }),
        CarrierMode::Trojan => Listener::Trojan(TrojanListener {
            enabled: true,
            remark: String::new(),
            listen: "0.0.0.0".parse()?,
            listen_port: 35000.try_into()?,
            endpoints,
            bind_interface: String::new(),
            socket: Default::default(),
            tcp: Default::default(),
            stream: Default::default(),
            password: None,
            tls,
        }),
        _ => return Err("streaming listener requires VLESS or Trojan".into()),
    })
}
