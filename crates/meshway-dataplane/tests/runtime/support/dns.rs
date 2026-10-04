use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RecordType},
};
use meshway_dns::{Answer, Begin, DiscoveryRequest, ExitSelection, Owner, Query, decode_query};

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
