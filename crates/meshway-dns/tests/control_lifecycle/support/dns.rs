use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question, ResponseCode},
    rr::{Name, RecordType},
};
use meshway_dns::{
    Answer, Begin, DiscoveryRequest, ExitSelection, Owner, Query, ResponseTransport, decode_query,
};

pub fn query(name: &str, kind: RecordType) -> Query {
    let mut message = Message::new(123, MessageType::Query, OpCode::Query);
    message.add_query(Question::query(Name::from_ascii(name).unwrap(), kind));
    decode_query(&message.to_vec().unwrap()).unwrap()
}

pub fn discover(owner: &mut Owner, name: &str, kind: RecordType) -> DiscoveryRequest {
    match owner.begin(query(name, kind), ExitSelection::NoExit) {
        Begin::DiscoveryRequest(request) => request,
        other => panic!("expected exact-name discovery, got {other:?}"),
    }
}

pub fn answer(result: Begin) -> Answer {
    match result {
        Begin::Answer(answer) => answer,
        other => panic!("expected bound answer, got {other:?}"),
    }
}

pub fn refused(result: Begin) {
    let answer = answer(result);
    answer.ensure_current().unwrap();
    let bytes = answer.encode(ResponseTransport::Udp, 1232).unwrap();
    let message = Message::from_vec(&bytes).unwrap();
    assert_eq!(message.metadata.response_code, ResponseCode::Refused);
    assert!(!message.metadata.authoritative);
    assert!(message.answers.is_empty() && message.authorities.is_empty());
}
