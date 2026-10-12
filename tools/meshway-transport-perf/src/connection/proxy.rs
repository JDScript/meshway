use crate::{Error, Protocol};
use meshway_model::VlessStream;
use meshway_transport::raw::Stream;
use meshway_transport::spec::{
    ClientTls, DialParameters, DialerStream, DialerTls, Endpoint, Listener, ListenerTls,
    ListenerWebSocket, Port, PreparedListener, SecretString, SocketOptions, TcpOptions, TlsMode,
    TlsVersion, TrojanDialer, TrojanListener, TrojanStream, VlessDialer, VlessListener,
};
use meshway_transport::tcp::TcpTransport;

pub(super) async fn open(
    protocol: Protocol,
    client: TcpTransport,
    server: TcpTransport,
    socket: &SocketOptions,
) -> Result<(Box<dyn Stream>, Box<dyn Stream>), Error> {
    // These are fresh benchmark-only protocol credentials, never live Node
    // state. Both comparison modes use exactly the same production providers.
    let name = "meshway-perf.invalid";
    let params = rcgen::CertificateParams::new(vec![name.into()])?;
    let key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    let websocket = matches!(protocol, Protocol::VlessWs);
    let alpn = if websocket {
        vec!["http/1.1".into()]
    } else {
        vec![]
    };
    let tls = ListenerTls {
        mode: TlsMode::Provided,
        server_name: name.into(),
        certificate: Some(SecretString::new(cert.pem())),
        private_key: Some(SecretString::new(key.serialize_pem()).into()),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: alpn.clone(),
        handshake_timeout: "5s".parse()?,
    };
    let client_tls = ClientTls {
        server_name: name.into(),
        certificate_chain: SecretString::new(cert.pem()),
        options: DialerTls {
            alpn,
            ..Default::default()
        },
    };
    let address = server.local_addr()?;
    let destination = Endpoint {
        host: "echo.meshway.invalid".into(),
        port: Port::new(443)?,
    };
    match protocol {
        Protocol::VlessTcp | Protocol::VlessWs => {
            let uuid = SecretString::new("11111111-1111-4111-8111-111111111111".into());
            let stream = if websocket {
                VlessStream::WebSocket
            } else {
                VlessStream::Tcp
            };
            let ws = websocket.then(|| ListenerWebSocket {
                path: "/meshway-perf".parse().expect("constant path"),
                host: Some(name.into()),
                ..Default::default()
            });
            let listener = PreparedListener::prepare(Listener::Vless(VlessListener {
                enabled: true,
                remark: String::new(),
                listen: address.ip(),
                listen_port: Port::new(address.port())?,
                endpoints: vec![],
                bind_interface: String::new(),
                socket: socket.clone(),
                tcp: TcpOptions::default(),
                stream,
                uuid: Some(uuid.clone().into()),
                flow: String::new(),
                tls,
                websocket: ws.clone(),
            }))?;
            let parameters = DialParameters::Vless {
                options: VlessDialer {
                    stream: if websocket {
                        DialerStream::WebSocket
                    } else {
                        DialerStream::Tcp
                    },
                    tls: client_tls.options.clone(),
                    ..Default::default()
                },
                stream,
                flow: String::new(),
                uuid,
                tls: client_tls,
                websocket: ws,
            };
            let connector = meshway_transport_stream::vless::Connector::new(&parameters)?;
            let acceptor = meshway_transport_stream::vless::Acceptor::new(&listener)?;
            let (connected, accepted) = tokio::try_join!(
                connector.connect(client, &destination),
                acceptor.accept(server)
            )?;
            if accepted.destination != destination {
                return Err("VLESS destination differs".into());
            }
            Ok((connected, accepted.stream))
        }
        Protocol::Trojan => {
            let password = SecretString::new("benchmark protocol password".into());
            let listener = PreparedListener::prepare(Listener::Trojan(TrojanListener {
                enabled: true,
                remark: String::new(),
                listen: address.ip(),
                listen_port: Port::new(address.port())?,
                endpoints: vec![],
                bind_interface: String::new(),
                socket: socket.clone(),
                tcp: TcpOptions::default(),
                stream: TrojanStream::Tcp,
                password: Some(password.clone().into()),
                tls,
            }))?;
            let parameters = DialParameters::Trojan {
                options: TrojanDialer {
                    tls: client_tls.options.clone(),
                    ..Default::default()
                },
                password,
                tls: client_tls,
            };
            let connector = meshway_transport_stream::trojan::Connector::new(&parameters)?;
            let acceptor = meshway_transport_stream::trojan::Acceptor::new(&listener)?;
            let (connected, accepted) = tokio::try_join!(
                connector.connect(client, &destination),
                acceptor.accept(server)
            )?;
            if accepted.destination != destination {
                return Err("Trojan destination differs".into());
            }
            Ok((connected, accepted.stream))
        }
        _ => Err("not a proxy benchmark protocol".into()),
    }
}
