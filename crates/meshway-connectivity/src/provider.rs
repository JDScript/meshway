use crate::Error;
use meshway_model::TransportProtocol;
use meshway_transport::raw::Stream;
use meshway_transport::spec::{DialParameters, Endpoint, Listener, Port, PreparedListener};

pub(crate) fn declaration(listener: &PreparedListener) -> (TransportProtocol, bool) {
    match listener.declaration() {
        Listener::NativeUdp(v) => (TransportProtocol::NativeUdp, v.enabled),
        Listener::NativeTcp(v) => (TransportProtocol::NativeTcp, v.enabled),
        Listener::Vless(v) => (TransportProtocol::Vless, v.enabled),
        Listener::Trojan(v) => (TransportProtocol::Trojan, v.enabled),
    }
}

fn service() -> Endpoint {
    Endpoint {
        host: "meshway.invalid".into(),
        port: Port::new(443).expect("nonzero constant"),
    }
}

pub(crate) enum Acceptor {
    Native,
    Vless(meshway_transport_vless::Acceptor),
    Trojan(meshway_transport_trojan::Acceptor),
}
impl Acceptor {
    pub fn new(listener: &PreparedListener) -> Result<Self, Error> {
        match declaration(listener) {
            (TransportProtocol::NativeTcp, true) => Ok(Self::Native),
            (TransportProtocol::Vless, true) => meshway_transport_vless::Acceptor::new(listener)
                .map(Self::Vless)
                .map_err(|_| Error::Configuration),
            (TransportProtocol::Trojan, true) => meshway_transport_trojan::Acceptor::new(listener)
                .map(Self::Trojan)
                .map_err(|_| Error::Configuration),
            _ => Err(Error::Configuration),
        }
    }
    pub async fn accept(&self, stream: Box<dyn Stream>) -> Result<Box<dyn Stream>, Error> {
        let (stream, destination) = match self {
            Self::Native => return Ok(stream),
            Self::Vless(value) => {
                let accepted = value.accept(stream).await.map_err(|_| Error::Provider)?;
                (accepted.stream, accepted.destination)
            }
            Self::Trojan(value) => {
                let accepted = value.accept(stream).await.map_err(|_| Error::Provider)?;
                (accepted.stream, accepted.destination)
            }
        };
        if destination != service() {
            return Err(Error::Provider);
        }
        Ok(stream)
    }
}

pub(crate) enum Connector {
    Native,
    Vless(meshway_transport_vless::Connector),
    Trojan(meshway_transport_trojan::Connector),
}
impl Connector {
    pub fn new(parameters: &DialParameters) -> Result<Self, Error> {
        match parameters {
            DialParameters::NativeTcp { options } if options.enabled => Ok(Self::Native),
            DialParameters::Vless { .. } => meshway_transport_vless::Connector::new(parameters)
                .map(Self::Vless)
                .map_err(|_| Error::Configuration),
            DialParameters::Trojan { .. } => meshway_transport_trojan::Connector::new(parameters)
                .map(Self::Trojan)
                .map_err(|_| Error::Configuration),
            _ => Err(Error::Configuration),
        }
    }
    pub async fn connect(&self, stream: Box<dyn Stream>) -> Result<Box<dyn Stream>, Error> {
        match self {
            Self::Native => Ok(stream),
            Self::Vless(value) => value
                .connect(stream, &service())
                .await
                .map_err(|_| Error::Provider),
            Self::Trojan(value) => value
                .connect(stream, &service())
                .await
                .map_err(|_| Error::Provider),
        }
    }
}
