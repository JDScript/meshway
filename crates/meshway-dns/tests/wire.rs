mod support;

use hickory_proto::{
    op::{Edns, Message, MessageType, OpCode, ResponseCode},
    rr::{DNSClass, Name, RData, Record, RecordType, rdata::TXT},
    serialize::binary::{BinDecodable, BinDecoder},
};
use meshway_dns::{ResponseTransport, WireError, decode_query, encode_response};
use support::query_message;

// ID=ABCD, RD=1, QDCOUNT=1, WWW.Example.COM IN A.
const DATAGRAM: &[u8] = &[
    0xab, 0xcd, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 3, b'W', b'W', b'W', 7,
    b'E', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'C', b'O', b'M', 0, 0, 1, 0, 1,
];

fn read_response(bytes: &[u8]) -> Message {
    let mut decoder = BinDecoder::new(bytes);
    let message = Message::read(&mut decoder).unwrap();
    assert!(decoder.is_empty());
    message
}

fn many_answers(query: &meshway_dns::Query, count: usize) -> Message {
    let mut response = query.response();
    for _ in 0..count {
        response.add_answer(Record::from_rdata(
            query.question().name().clone(),
            25,
            RData::TXT(TXT::new(vec!["x".repeat(100)])),
        ));
    }
    response
}

#[test]
fn literal_datagram_preserves_client_id_case_and_question() {
    let query = decode_query(DATAGRAM).unwrap();
    assert_eq!(query.id(), 0xabcd);
    assert_eq!(query.question().name().to_ascii(), "WWW.Example.COM.");
    let mut response = query.response();
    response.metadata.id = 42;
    response.queries[0].name = Name::from_ascii("www.example.com.").unwrap();
    let bytes = encode_response(&query, &response, ResponseTransport::Udp, 1232).unwrap();
    assert_eq!(&bytes[12..], &DATAGRAM[12..]);
    let decoded = read_response(&bytes);
    assert_eq!(decoded.metadata.id, 0xabcd);
    assert_eq!(decoded.metadata.message_type, MessageType::Response);
    assert!(decoded.metadata.recursion_desired);
    assert!(!decoded.metadata.recursion_available);
    assert!(!decoded.metadata.authoritative);
    assert!(!decoded.metadata.authentic_data);
    assert_eq!(decoded.metadata.response_code, ResponseCode::NoError);
}

#[test]
fn malformed_compression_and_trailing_data_are_rejected() {
    let mut trailing = DATAGRAM.to_vec();
    trailing.push(0);
    assert!(matches!(
        decode_query(&trailing),
        Err(WireError::TrailingBytes)
    ));
    for name in [
        vec![0xc0, 0x0c],
        vec![0xc0, 0xff],
        vec![0x40, 0],
        vec![63, b'a'],
    ] {
        let mut bytes = DATAGRAM[..12].to_vec();
        bytes.extend(name);
        bytes.extend([0, 1, 0, 1]);
        assert!(matches!(decode_query(&bytes), Err(WireError::Decode(_))));
    }
    assert!(matches!(
        decode_query(&DATAGRAM[..11]),
        Err(WireError::MessageLength)
    ));
    assert!(matches!(
        decode_query(&vec![0; 65536]),
        Err(WireError::MessageLength)
    ));
    assert!(decode_query(&DATAGRAM[..DATAGRAM.len() - 1]).is_err());
}

#[test]
fn compressed_root_in_opt_is_decoded_with_full_consumption() {
    let mut bytes = DATAGRAM.to_vec();
    bytes[11] = 1;
    // Compressed OPT owner points to the question's root label at offset 28.
    bytes.extend([0xc0, 28, 0, 41, 4, 208, 0, 0, 0, 0, 0, 0]);
    let query = decode_query(&bytes).unwrap();
    assert_eq!(query.edns().unwrap().max_payload(), 1232);
    bytes.push(0);
    assert!(matches!(
        decode_query(&bytes),
        Err(WireError::TrailingBytes)
    ));
    bytes.pop();
    // The same compression mechanism cannot give OPT a non-root owner.
    bytes[DATAGRAM.len() + 1] = 12;
    assert!(matches!(decode_query(&bytes), Err(WireError::Decode(_))));
}

#[test]
fn edns_options_are_parsed_but_not_reflected_into_local_responses() {
    let mut bytes = DATAGRAM.to_vec();
    bytes[11] = 1;
    // OPT, payload 1232, RDLEN 7; private-use option 65001, length 3, "abc".
    bytes.extend([
        0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 7, 0xfd, 0xe9, 0, 3, b'a', b'b', b'c',
    ]);
    let query = decode_query(&bytes).unwrap();
    assert_eq!(query.edns().unwrap().options().as_ref().len(), 1);
    let response =
        encode_response(&query, &query.response(), ResponseTransport::Udp, 1232).unwrap();
    assert!(
        read_response(&response)
            .edns
            .unwrap()
            .options()
            .as_ref()
            .is_empty()
    );
    bytes.pop();
    assert!(matches!(decode_query(&bytes), Err(WireError::Decode(_))));
}

#[test]
fn query_shape_counts_flags_classes_and_meta_types_are_checked() {
    for (index, byte) in [
        (2, 0x81),
        (2, 0x05),
        (2, 0x03),
        (3, 0x40),
        (3, 0x80),
        (3, 1),
        (5, 0),
        (5, 2),
        (7, 1),
        (9, 1),
        (11, 2),
    ] {
        let mut bytes = DATAGRAM.to_vec();
        bytes[index] = byte;
        assert!(matches!(decode_query(&bytes), Err(WireError::QueryShape)));
    }
    let mut message = query_message("example.com.", RecordType::A);
    message.metadata.op_code = OpCode::Update;
    assert!(matches!(
        decode_query(&message.to_vec().unwrap()),
        Err(WireError::QueryShape)
    ));
    message.metadata.op_code = OpCode::Query;
    message.queries[0].query_class = DNSClass::CH;
    assert!(matches!(
        decode_query(&message.to_vec().unwrap()),
        Err(WireError::UnsupportedClass)
    ));
    for kind in [
        RecordType::AXFR,
        RecordType::IXFR,
        RecordType::OPT,
        RecordType::TSIG,
        RecordType::ZERO,
    ] {
        let message = query_message("example.com.", kind);
        assert!(matches!(
            decode_query(&message.to_vec().unwrap()),
            Err(WireError::UnsupportedQuestionType)
        ));
    }
    // Ordinary non-address query types remain usable by resolver selection.
    assert!(
        decode_query(
            &query_message("_sip._tcp.example.com.", RecordType::SRV)
                .to_vec()
                .unwrap()
        )
        .is_ok()
    );
}

#[test]
fn unsupported_edns_and_non_opt_additionals_are_rejected() {
    let mut message = query_message("example.com.", RecordType::A);
    let mut edns = Edns::new();
    edns.set_version(1);
    message.edns = Some(edns);
    assert!(matches!(
        decode_query(&message.to_vec().unwrap()),
        Err(WireError::EdnsVersion(1))
    ));
    message.edns = None;
    message.add_additional(Record::from_rdata(
        Name::root(),
        0,
        RData::TXT(TXT::new(vec!["extra".into()])),
    ));
    assert!(matches!(
        decode_query(&message.to_vec().unwrap()),
        Err(WireError::QueryShape)
    ));
    let mut duplicate = DATAGRAM.to_vec();
    duplicate[11] = 2;
    for _ in 0..2 {
        duplicate.extend([0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 0]);
    }
    assert!(matches!(
        decode_query(&duplicate),
        Err(WireError::QueryShape)
    ));
}

#[test]
fn udp_truncates_whole_rrsets_while_tcp_keeps_the_complete_answer() {
    let query = decode_query(
        &query_message("example.com.", RecordType::TXT)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    let response = many_answers(&query, 12);
    let udp = encode_response(&query, &response, ResponseTransport::Udp, 1232).unwrap();
    assert!(udp.len() <= 512);
    let udp = read_response(&udp);
    assert!(udp.metadata.truncation);
    assert!(udp.answers.is_empty());
    assert_eq!(udp.queries, vec![query.question().clone()]);
    assert!(udp.edns.is_none());
    let tcp = encode_response(&query, &response, ResponseTransport::Tcp, 1232).unwrap();
    assert!(tcp.len() > 512);
    let tcp = read_response(&tcp);
    assert!(!tcp.metadata.truncation);
    assert_eq!(tcp.answers.len(), 12);
    assert!(matches!(
        encode_response(
            &query,
            &many_answers(&query, 800),
            ResponseTransport::Tcp,
            1232
        ),
        Err(WireError::ResponseTooLarge)
    ));
}

#[test]
fn edns_payload_is_clamped_and_opt_survives_truncation() {
    let mut message = query_message("example.com.", RecordType::TXT);
    message.metadata.authentic_data = true;
    message.metadata.checking_disabled = true;
    let mut edns = Edns::new();
    edns.set_max_payload(4096).set_dnssec_ok(true);
    message.edns = Some(edns);
    let query = decode_query(&message.to_vec().unwrap()).unwrap();
    let response = many_answers(&query, 8);
    let fits = encode_response(&query, &response, ResponseTransport::Udp, 1232).unwrap();
    assert!(fits.len() > 512 && fits.len() <= 1232);
    let fits = read_response(&fits);
    assert_eq!(fits.answers.len(), 8);
    assert!(!fits.metadata.truncation);
    assert!(!fits.metadata.authentic_data);
    assert!(fits.metadata.checking_disabled);
    let truncated = encode_response(&query, &response, ResponseTransport::Udp, 512).unwrap();
    assert!(truncated.len() <= 512);
    let truncated = read_response(&truncated);
    assert!(truncated.metadata.truncation);
    assert!(truncated.answers.is_empty());
    let opt = truncated.edns.unwrap();
    assert_eq!(opt.max_payload(), 512);
    assert!(opt.flags().dnssec_ok);
    assert_eq!(opt.version(), 0);
    assert!(matches!(
        encode_response(&query, &response, ResponseTransport::Udp, 511),
        Err(WireError::PayloadLimit)
    ));

    // Force a sub-512 advertised size on the wire; Hickory's builder clamps it.
    let mut raw = message.to_vec().unwrap();
    let opt_offset = raw.len() - 11;
    raw[opt_offset + 3..opt_offset + 5].copy_from_slice(&1_u16.to_be_bytes());
    let small = decode_query(&raw).unwrap();
    let encoded = encode_response(&small, &small.response(), ResponseTransport::Udp, 1232).unwrap();
    assert!(!read_response(&encoded).metadata.truncation);
    let truncated = encode_response(
        &small,
        &many_answers(&small, 8),
        ResponseTransport::Udp,
        1232,
    )
    .unwrap();
    assert!(read_response(&truncated).metadata.truncation);
    assert!(truncated.len() <= 512);
}

#[test]
fn response_question_mismatch_and_unrepresentable_rcode_are_rejected() {
    let query = decode_query(DATAGRAM).unwrap();
    let mut response = query.response();
    response.queries[0].name = Name::from_ascii("another.example.").unwrap();
    assert!(matches!(
        encode_response(&query, &response, ResponseTransport::Udp, 1232),
        Err(WireError::ResponseShape)
    ));
    let mut response = query.response();
    response.metadata.response_code = ResponseCode::BADVERS;
    assert!(matches!(
        encode_response(&query, &response, ResponseTransport::Udp, 1232),
        Err(WireError::ExtendedResponseCode)
    ));
    response.metadata.response_code = ResponseCode::ServFail;
    let bytes = encode_response(&query, &response, ResponseTransport::Udp, 1232).unwrap();
    assert_eq!(
        read_response(&bytes).metadata.response_code,
        ResponseCode::ServFail
    );
}
