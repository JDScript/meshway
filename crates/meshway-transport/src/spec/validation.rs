use crate::spec::*;
use meshway_model::VlessStream;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use utoipa::{PartialSchema, ToSchema};

/// Why a listener or dialer declaration is invalid. `code()` is the stable
/// wire spelling clients localize; `Display` is the English rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeclarationError {
    #[error("invalid host")]
    InvalidHost,
    #[error("keepAlive.count must be positive")]
    KeepAliveCount,
    #[error("TLS version order")]
    TlsVersionOrder,
    #[error("invalid ALPN")]
    InvalidAlpn,
    #[error("provided TLS requires certificate and privateKey")]
    ProvidedTlsIncomplete,
    #[error("empty TLS material")]
    EmptyTlsMaterial,
    #[error("VLESS flow must be empty")]
    VlessFlow,
    #[error("invalid VLESS UUID")]
    InvalidVlessUuid,
    #[error("TCP stream cannot use websocket")]
    TcpStreamWebsocket,
    #[error("transport listed more than once")]
    RepeatedTransport,
    #[error("WebSocket requires HTTP/1.1 ALPN")]
    WebsocketAlpn,
    #[error("empty Trojan password")]
    EmptyTrojanPassword,
    #[error("listener material preparation required")]
    MaterialRequired,
    #[error("localAddress must be an IP or empty")]
    LocalAddress,
}
impl DeclarationError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidHost => "invalid_host",
            Self::KeepAliveCount => "keep_alive_count",
            Self::TlsVersionOrder => "tls_version_order",
            Self::InvalidAlpn => "invalid_alpn",
            Self::ProvidedTlsIncomplete => "provided_tls_incomplete",
            Self::EmptyTlsMaterial => "empty_tls_material",
            Self::VlessFlow => "vless_flow",
            Self::InvalidVlessUuid => "invalid_vless_uuid",
            Self::TcpStreamWebsocket => "tcp_stream_websocket",
            Self::RepeatedTransport => "repeated_transport",
            Self::WebsocketAlpn => "websocket_alpn",
            Self::EmptyTrojanPassword => "empty_trojan_password",
            Self::MaterialRequired => "material_required",
            Self::LocalAddress => "local_address",
        }
    }
}

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MaterialRequirement {
    Uuid,
    Password,
    Certificate,
    PrivateKey,
    CertificateValidation,
}
}

/// Strict DNS host normalization, without URL/port/path parsing.
pub fn normalize_host(host: &str) -> Result<String, DeclarationError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    let ascii = idna::domain_to_ascii_strict(host.strip_suffix('.').unwrap_or(host))
        .map_err(|_| DeclarationError::InvalidHost)?
        .to_ascii_lowercase();
    let ascii = ascii.as_str();
    if ascii.is_empty()
        || ascii.len() > 253
        || ascii.split('.').any(|s| {
            s.is_empty()
                || s.len() > 63
                || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || s.starts_with('-')
                || s.ends_with('-')
        })
    {
        return Err(DeclarationError::InvalidHost);
    }
    Ok(ascii.to_owned())
}

fn tcp(options: &TcpOptions) -> Result<(), DeclarationError> {
    if options.keep_alive.count == 0 {
        return Err(DeclarationError::KeepAliveCount);
    }
    Ok(())
}

/// Validate the strict hyphenated VLESS UUID spelling. Hex digits may use either
/// case; braces, URNs and the compact UUID form are not declaration syntax.
/// This checks syntax only, without decoding or retaining credential material.
pub fn validate_vless_uuid(value: &str) -> Result<(), DeclarationError> {
    if value.len() != 36
        || !value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(DeclarationError::InvalidVlessUuid);
    }
    Ok(())
}
fn tls(min: TlsVersion, max: TlsVersion, alpn: &[String]) -> Result<(), DeclarationError> {
    crate::spec::tls::validate_tls_versions(min, max)
        .map_err(|_| DeclarationError::TlsVersionOrder)?;
    if alpn
        .iter()
        .any(|s| s.is_empty() || s.len() > 255 || s.chars().any(char::is_control))
    {
        return Err(DeclarationError::InvalidAlpn);
    }
    Ok(())
}
fn listener_tls(value: &mut ListenerTls) -> Result<(), DeclarationError> {
    value.server_name = normalize_host(&value.server_name)?;
    tls(value.min_version, value.max_version, &value.alpn)?;
    if value.mode == TlsMode::Provided
        && (value.certificate.is_none() || value.private_key.is_none())
    {
        return Err(DeclarationError::ProvidedTlsIncomplete);
    }
    if value
        .certificate
        .as_ref()
        .is_some_and(|v| v.expose().is_empty())
        || value
            .private_key
            .as_ref()
            .is_some_and(|v| v.expose().is_empty())
    {
        return Err(DeclarationError::EmptyTlsMaterial);
    }
    Ok(())
}
impl Listener {
    pub fn normalize(&mut self) -> Result<(), DeclarationError> {
        let endpoints = match self {
            Self::NativeUdp(v) => &mut v.endpoints,
            Self::NativeTcp(v) => {
                tcp(&v.tcp)?;
                &mut v.endpoints
            }
            Self::Vless(v) => {
                tcp(&v.tcp)?;
                listener_tls(&mut v.tls)?;
                if !v.flow.is_empty() {
                    return Err(DeclarationError::VlessFlow);
                }
                if let Some(uuid) = &v.uuid {
                    validate_vless_uuid(uuid.expose())?;
                }
                match v.stream {
                    VlessStream::Tcp if v.websocket.is_some() => {
                        return Err(DeclarationError::TcpStreamWebsocket);
                    }
                    VlessStream::WebSocket => {
                        validate_websocket_alpn(&v.tls.alpn)
                            .map_err(|_| DeclarationError::WebsocketAlpn)?;
                        let ws = v.websocket.get_or_insert_with(ListenerWebSocket::default);
                        ws.host = Some(normalize_host(
                            ws.host.as_deref().unwrap_or(&v.tls.server_name),
                        )?);
                    }
                    _ => {}
                }
                &mut v.endpoints
            }
            Self::Trojan(v) => {
                tcp(&v.tcp)?;
                listener_tls(&mut v.tls)?;
                if v.password.as_ref().is_some_and(|s| s.expose().is_empty()) {
                    return Err(DeclarationError::EmptyTrojanPassword);
                }
                &mut v.endpoints
            }
        };
        for endpoint in endpoints {
            endpoint.host = normalize_host(&endpoint.host)?;
        }
        Ok(())
    }

    /// Generation and certificate/key/name/expiry/revocation verification are
    /// explicit Controller responsibilities, never guessed from nonempty PEM.
    pub fn material_requirements(&self) -> Vec<MaterialRequirement> {
        let mut result = Vec::new();
        let tls = match self {
            Self::NativeUdp(_) | Self::NativeTcp(_) => return result,
            Self::Vless(v) => {
                if v.uuid.is_none() {
                    result.push(MaterialRequirement::Uuid);
                }
                &v.tls
            }
            Self::Trojan(v) => {
                if v.password.is_none() {
                    result.push(MaterialRequirement::Password);
                }
                &v.tls
            }
        };
        if tls.certificate.is_none() {
            result.push(MaterialRequirement::Certificate);
        }
        if tls.private_key.is_none() {
            result.push(MaterialRequirement::PrivateKey);
        }
        result.push(MaterialRequirement::CertificateValidation);
        result
    }
}

/// A structurally complete, normalized listener, without a TLS verification claim.
/// The Server must verify certificate/key/name/expiry/revocation for this exact
/// declaration before accepting it into an executable configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PreparedListener(Listener);
impl utoipa::PartialSchema for PreparedListener {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        crate::spec::settings::listener_schema(true)
    }
}
impl ToSchema for PreparedListener {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        schemas.push(("Listener".into(), Listener::schema()));
        Listener::schemas(schemas);
    }
}
impl<'de> Deserialize<'de> for PreparedListener {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::prepare(Listener::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
impl PreparedListener {
    pub fn prepare(mut listener: Listener) -> Result<Self, DeclarationError> {
        listener.normalize()?;
        if listener
            .material_requirements()
            .iter()
            .any(|r| *r != MaterialRequirement::CertificateValidation)
        {
            return Err(DeclarationError::MaterialRequired);
        }
        Ok(Self(listener))
    }
    pub fn declaration(&self) -> &Listener {
        &self.0
    }
}

impl DialerSet {
    pub fn validate(&self) -> Result<(), DeclarationError> {
        fn local(value: &str) -> Result<(), DeclarationError> {
            if !value.is_empty() && value.parse::<IpAddr>().is_err() {
                return Err(DeclarationError::LocalAddress);
            }
            Ok(())
        }
        if let Some(v) = &self.native_udp {
            local(&v.local_address)?;
        }
        if let Some(v) = &self.native_tcp {
            local(&v.local_address)?;
            tcp(&v.tcp)?;
        }
        if let Some(v) = &self.vless {
            local(&v.local_address)?;
            tcp(&v.tcp)?;
            tls(v.tls.min_version, v.tls.max_version, &v.tls.alpn)?;
            if v.stream == DialerStream::Tcp && v.websocket.is_some() {
                return Err(DeclarationError::TcpStreamWebsocket);
            }
            if v.stream == DialerStream::WebSocket || v.websocket.is_some() {
                validate_websocket_alpn(&v.tls.alpn)
                    .map_err(|_| DeclarationError::WebsocketAlpn)?;
            }
        }
        if let Some(v) = &self.trojan {
            local(&v.local_address)?;
            tcp(&v.tcp)?;
            tls(v.tls.min_version, v.tls.max_version, &v.tls.alpn)?;
        }
        Ok(())
    }
}
