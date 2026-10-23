//! The nonce UDP→TCP probe server that proves local link DNS behavior.
use crate::system::linux::SystemDnsError;

/// `reached` is set once any UDP datagram arrives: resolved sent the query
/// to the owned link's server at all.
pub(super) async fn serve_probe(
    udp: tokio::net::UdpSocket,
    tcp: tokio::net::TcpListener,
    expected: &str,
    reached: &std::sync::atomic::AtomicBool,
) -> Result<(), SystemDnsError> {
    use hickory_proto::rr::{Name, RData, Record, RecordType, rdata};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut expected = Name::from_ascii(expected).map_err(|_| SystemDnsError::Backend)?;
    expected.set_fqdn(true);
    let mut buffer = [0u8; 4096];
    let (size, peer) = udp
        .recv_from(&mut buffer)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    reached.store(true, std::sync::atomic::Ordering::Relaxed);
    let query = crate::decode_query(&buffer[..size]).map_err(|_| SystemDnsError::Backend)?;
    if !peer.ip().is_loopback()
        || query.question().name() != &expected
        || query.question().query_type() != RecordType::A
    {
        return Err(SystemDnsError::Backend);
    }
    let mut response = query.response();
    response.metadata.truncation = true;
    let bytes = crate::encode_response(&query, &response, crate::ResponseTransport::Udp, 4096)
        .map_err(|_| SystemDnsError::Backend)?;
    udp.send_to(&bytes, peer)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let (mut stream, peer) = tcp.accept().await.map_err(|_| SystemDnsError::Backend)?;
    if !peer.ip().is_loopback() {
        return Err(SystemDnsError::Backend);
    }
    let size = stream
        .read_u16()
        .await
        .map_err(|_| SystemDnsError::Backend)? as usize;
    if !(12..=buffer.len()).contains(&size) {
        return Err(SystemDnsError::Backend);
    }
    stream
        .read_exact(&mut buffer[..size])
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let query = crate::decode_query(&buffer[..size]).map_err(|_| SystemDnsError::Backend)?;
    if query.question().name() != &expected || query.question().query_type() != RecordType::A {
        return Err(SystemDnsError::Backend);
    }
    let mut response = query.response();
    response.add_answer(Record::from_rdata(
        expected,
        0,
        RData::A(rdata::A(std::net::Ipv4Addr::LOCALHOST)),
    ));
    let bytes = crate::encode_response(&query, &response, crate::ResponseTransport::Tcp, 4096)
        .map_err(|_| SystemDnsError::Backend)?;
    stream
        .write_u16(bytes.len() as u16)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    Ok(())
}
