//! VLESS-specific address tags and port-before-address order; see
//! mihomo/transport/vless/{vless,conn}.go (88dcbf7).
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use meshway_transport::spec::{Endpoint, Port, normalize_host, validate_vless_uuid};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

use crate::Error;

pub(super) fn uuid(value: &str) -> Result<Zeroizing<[u8; 16]>, Error> {
    validate_vless_uuid(value).map_err(|_| Error::Configuration)?;
    uuid::Uuid::parse_str(value)
        .map(|uuid| Zeroizing::new(uuid.into_bytes()))
        .map_err(|_| Error::Configuration)
}

pub(super) fn request(uuid: &[u8; 16], endpoint: &Endpoint) -> Result<Zeroizing<Vec<u8>>, Error> {
    let host = normalize_host(&endpoint.host).map_err(|_| Error::Configuration)?;
    let mut request = Zeroizing::new(Vec::with_capacity(276));
    request.push(0);
    request.extend_from_slice(uuid);
    request.extend_from_slice(&[0, 1]); // no addons, TCP CONNECT
    request.extend_from_slice(&endpoint.port.get().to_be_bytes());
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(3);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.push(2);
            request.push(u8::try_from(host.len()).map_err(|_| Error::Configuration)?);
            request.extend_from_slice(host.as_bytes());
        }
    }
    Ok(request)
}

pub(super) async fn accept<S: AsyncRead + Unpin>(
    stream: &mut S,
    uuid: &[u8; 16],
) -> Result<Endpoint, Error> {
    if stream.read_u8().await? != 0 {
        return Err(Error::Protocol);
    }
    let mut supplied = Zeroizing::new([0; 16]);
    stream.read_exact(&mut supplied[..]).await?;
    if !bool::from(supplied[..].ct_eq(&uuid[..])) {
        return Err(Error::Credential);
    }
    if stream.read_u8().await? != 0 || stream.read_u8().await? != 1 {
        return Err(Error::Protocol);
    }
    let port = Port::new(stream.read_u16().await?).map_err(|_| Error::Protocol)?;
    let host = match stream.read_u8().await? {
        1 => {
            let mut octets = [0; 4];
            stream.read_exact(&mut octets).await?;
            Ipv4Addr::from(octets).to_string()
        }
        3 => {
            let mut octets = [0; 16];
            stream.read_exact(&mut octets).await?;
            Ipv6Addr::from(octets).to_string()
        }
        2 => {
            let length = usize::from(stream.read_u8().await?);
            let mut name = Zeroizing::new([0; 255]);
            stream.read_exact(&mut name[..length]).await?;
            let host = std::str::from_utf8(&name[..length]).map_err(|_| Error::Protocol)?;
            normalize_host(host).map_err(|_| Error::Protocol)?
        }
        _ => return Err(Error::Protocol),
    };
    Ok(Endpoint { host, port })
}

#[cfg(test)]
mod tests;
