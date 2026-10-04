mod support;
use hickory_proto::{
    op::{Edns, Message, ResponseCode},
    rr::{RData, Record, RecordType, rdata},
};
use meshway_dns::{ResponseTransport, TrustedUpstreamResponse, decode_query};
use support::query_message;

#[test]
fn upstream_preserves_rcode_and_whole_rrsets_with_canonical_truncation() {
    let query = decode_query(
        &query_message("example.com.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    let mut message = query.response();
    message.metadata.recursion_available = true;
    for _ in 0..80 {
        message.add_answer(Record::from_rdata(
            query.question().name().clone(),
            60,
            RData::A(rdata::A("192.0.2.1".parse().unwrap())),
        ));
    }
    let response = TrustedUpstreamResponse::decode(
        &query,
        &message.to_vec().unwrap(),
        ResponseTransport::Tcp,
        1232,
    )
    .unwrap();
    let udp = Message::from_vec(
        &response
            .encode(&query, ResponseTransport::Udp, 1232)
            .unwrap(),
    )
    .unwrap();
    assert!(udp.metadata.truncation);
    assert!(udp.answers.is_empty());
    let tcp = Message::from_vec(
        &response
            .encode(&query, ResponseTransport::Tcp, 1232)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(tcp.answers.len(), 80);
    assert_eq!(tcp.answers[0].ttl, 60);
    message.answers.clear();
    message.metadata.response_code = ResponseCode::NXDomain;
    let response = TrustedUpstreamResponse::decode(
        &query,
        &message.to_vec().unwrap(),
        ResponseTransport::Udp,
        1232,
    )
    .unwrap();
    let encoded = response
        .encode(&query, ResponseTransport::Udp, 1232)
        .unwrap();
    assert_eq!(
        Message::from_vec(&encoded).unwrap().metadata.response_code,
        ResponseCode::NXDomain
    );
}

#[test]
fn rejects_wrong_transaction_question_flags_edns_trailing_and_oversized_udp() {
    let query = decode_query(
        &query_message("example.com.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .unwrap();
    for case in 0..8 {
        let mut message = query.response();
        match case {
            0 => message.metadata.id ^= 1,
            1 => {
                message.queries[0].set_query_type(RecordType::AAAA);
            }
            2 => {
                message.queries.push(query.question().clone());
            }
            3 => {
                message.edns = Some(Edns::new());
            }
            _ => {}
        }
        let mut bytes = message.to_vec().unwrap();
        match case {
            4 => bytes[3] |= 0x40,
            5 => bytes.push(0),
            6 => bytes.resize(513, 0),
            7 => bytes[2] &= 0x7f,
            _ => {}
        }
        assert!(
            TrustedUpstreamResponse::decode(&query, &bytes, ResponseTransport::Udp, 1232).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn accepts_negotiated_edns_clears_ad_and_does_not_synthesize_authority() {
    let mut request = query_message("example.com.", RecordType::A);
    let mut edns = Edns::new();
    edns.set_max_payload(1232).set_dnssec_ok(true);
    request.edns = Some(edns.clone());
    let query = decode_query(&request.to_vec().unwrap()).unwrap();
    let mut message = query.response();
    message.edns = Some(edns);
    message.metadata.authentic_data = true;
    let validated = TrustedUpstreamResponse::decode(
        &query,
        &message.to_vec().unwrap(),
        ResponseTransport::Udp,
        1232,
    )
    .unwrap();
    let encoded = validated
        .encode(&query, ResponseTransport::Udp, 1232)
        .unwrap();
    let result = Message::from_vec(&encoded).unwrap();
    assert!(!result.metadata.authentic_data && !result.metadata.authoritative);
    assert!(result.edns.is_some());
    message.edns.as_mut().unwrap().flags_mut().z = 1;
    assert!(
        TrustedUpstreamResponse::decode(
            &query,
            &message.to_vec().unwrap(),
            ResponseTransport::Udp,
            1232
        )
        .is_err()
    );
}
