//! Wire order follows sing-box/transport/trojan/protocol.go (927770c).
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use meshway_transport_spec::{Endpoint, Port, normalize_host};
use sha2::{Digest, Sha224};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

use crate::Error;

pub(super) fn key(password: &str) -> Result<Zeroizing<[u8; 56]>, Error> {
    if password.is_empty() {
        return Err(Error::Configuration);
    }
    let hash = Zeroizing::new(<[u8; 28]>::from(Sha224::digest(password.as_bytes())));
    let mut key = Zeroizing::new([0; 56]);
    hex::encode_to_slice(&hash[..], &mut key[..]).map_err(|_| Error::Configuration)?;
    Ok(key)
}

pub(super) fn request(key: &[u8; 56], endpoint: &Endpoint) -> Result<Zeroizing<Vec<u8>>, Error> {
    let host = normalize_host(&endpoint.host).map_err(|_| Error::Configuration)?;
    // Maximum request: 56 + CRLF + CMD + ATYP + LEN + 253 + PORT + CRLF.
    let mut request = Zeroizing::new(Vec::with_capacity(318));
    request.extend_from_slice(key);
    request.extend_from_slice(b"\r\n\x01");
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.push(3);
            request.push(u8::try_from(host.len()).map_err(|_| Error::Configuration)?);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&endpoint.port.get().to_be_bytes());
    request.extend_from_slice(b"\r\n");
    Ok(request)
}

pub(super) async fn accept<S: AsyncRead + Unpin>(
    stream: &mut S,
    key: &[u8; 56],
) -> Result<Endpoint, Error> {
    let mut supplied = Zeroizing::new([0; 56]);
    stream.read_exact(&mut supplied[..]).await?;
    if !bool::from(supplied[..].ct_eq(&key[..])) {
        return Err(Error::Credential);
    }
    crlf(stream).await?;
    if stream.read_u8().await? != 1 {
        return Err(Error::Protocol);
    }
    let host = match stream.read_u8().await? {
        1 => {
            let mut octets = [0; 4];
            stream.read_exact(&mut octets).await?;
            Ipv4Addr::from(octets).to_string()
        }
        4 => {
            let mut octets = [0; 16];
            stream.read_exact(&mut octets).await?;
            Ipv6Addr::from(octets).to_string()
        }
        3 => {
            let length = usize::from(stream.read_u8().await?);
            let mut name = Zeroizing::new([0; 255]);
            stream.read_exact(&mut name[..length]).await?;
            let host = std::str::from_utf8(&name[..length]).map_err(|_| Error::Protocol)?;
            normalize_host(host).map_err(|_| Error::Protocol)?
        }
        _ => return Err(Error::Protocol),
    };
    let port = Port::new(stream.read_u16().await?).map_err(|_| Error::Protocol)?;
    crlf(stream).await?;
    Ok(Endpoint { host, port })
}

async fn crlf<S: AsyncRead + Unpin>(stream: &mut S) -> Result<(), Error> {
    let mut marker = [0; 2];
    stream.read_exact(&mut marker).await?;
    if marker != *b"\r\n" {
        return Err(Error::Protocol);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
