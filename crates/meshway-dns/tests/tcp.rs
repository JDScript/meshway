mod support;

use bytes::{Bytes, BytesMut};
use hickory_proto::{op::Message, rr::RecordType};
use meshway_dns::{
    ResponseTransport, TcpCodec, TcpError, WireError, decode_query, encode_response,
};
use support::query_message;
use tokio_util::codec::{Decoder, Encoder};

fn frame(payload: &[u8]) -> BytesMut {
    let mut bytes = BytesMut::new();
    bytes.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn every_two_fragment_boundary_and_coalesced_frames_decode() {
    let first = query_message("MiXeD.example.", RecordType::A)
        .to_vec()
        .unwrap();
    let mut second = query_message("other.example.", RecordType::AAAA);
    second.metadata.id = 321;
    let second = second.to_vec().unwrap();
    let mut joined = frame(&first);
    joined.extend_from_slice(&frame(&second));
    for split in 0..=joined.len() {
        let mut codec = TcpCodec::new(2048).unwrap();
        let mut buffer = BytesMut::new();
        let mut queries = Vec::new();
        for fragment in [&joined[..split], &joined[split..]] {
            buffer.extend_from_slice(fragment);
            while let Some(query) = codec.decode(&mut buffer).unwrap() {
                queries.push(query);
            }
        }
        assert_eq!(queries.len(), 2, "split={split}");
        assert_eq!(queries[0].question().name().to_ascii(), "MiXeD.example.");
        assert_eq!(queries[1].id(), 321);
        assert!(codec.decode_eof(&mut buffer).unwrap().is_none());
    }
}

#[test]
fn partial_prefix_body_and_consumed_prefix_are_errors_at_eof() {
    let payload = query_message("example.", RecordType::A).to_vec().unwrap();
    let framed = frame(&payload);
    for length in 1..framed.len() {
        let mut codec = TcpCodec::new(512).unwrap();
        let mut buffer = BytesMut::from(&framed[..length]);
        assert!(codec.decode(&mut buffer).unwrap().is_none());
        assert!(
            matches!(codec.decode_eof(&mut buffer), Err(TcpError::Framing(_))),
            "length={length}"
        );
    }
    let mut codec = TcpCodec::new(512).unwrap();
    assert!(codec.decode_eof(&mut BytesMut::new()).unwrap().is_none());
    let mut complete = frame(&payload);
    assert!(codec.decode_eof(&mut complete).unwrap().is_some());
    assert!(codec.decode_eof(&mut complete).unwrap().is_none());
}

#[test]
fn size_limits_apply_to_declared_length_and_outbound_payload() {
    assert!(matches!(TcpCodec::new(11), Err(WireError::MessageLength)));
    let mut codec = TcpCodec::new(512).unwrap();
    let mut oversized_prefix = BytesMut::from(&513_u16.to_be_bytes()[..]);
    assert!(matches!(
        codec.decode(&mut oversized_prefix),
        Err(TcpError::Framing(_))
    ));
    let mut codec = TcpCodec::new(512).unwrap();
    assert!(matches!(
        codec.decode(&mut BytesMut::from(&[0, 0][..])),
        Err(TcpError::Wire(WireError::MessageLength))
    ));
    let mut codec = TcpCodec::new(512).unwrap();
    let mut out = BytesMut::new();
    assert!(codec.encode(Bytes::from(vec![0; 513]), &mut out).is_err());
    assert!(out.is_empty());
    assert!(
        codec
            .encode(Bytes::from_static(&[0; 11]), &mut out)
            .is_err()
    );
    assert!(out.is_empty());
}

#[test]
fn each_frame_must_contain_exactly_one_valid_dns_query() {
    let mut payload = query_message("example.", RecordType::A).to_vec().unwrap();
    payload.push(0);
    let mut codec = TcpCodec::new(512).unwrap();
    assert!(matches!(
        codec.decode(&mut frame(&payload)),
        Err(TcpError::Wire(WireError::TrailingBytes))
    ));
}

#[test]
fn encoder_writes_network_order_length_and_unmodified_response_payload() {
    let query = decode_query(
        &query_message("example.", RecordType::AAAA)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    let payload = encode_response(&query, &query.response(), ResponseTransport::Tcp, 1232).unwrap();
    let mut codec = TcpCodec::new(u16::MAX).unwrap();
    let mut output = BytesMut::new();
    codec.encode(payload.clone(), &mut output).unwrap();
    assert_eq!(
        usize::from(u16::from_be_bytes([output[0], output[1]])),
        payload.len()
    );
    assert_eq!(&output[2..], &payload[..]);
    assert_eq!(
        Message::from_vec(&output[2..]).unwrap().metadata.id,
        query.id()
    );
}
