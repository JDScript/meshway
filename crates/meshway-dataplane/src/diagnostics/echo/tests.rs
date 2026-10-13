use super::*;
use crate::IcmpError;

const SOURCE: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 1);
const TARGET: Ipv4Addr = Ipv4Addr::new(1, 2, 4, 8);
const ROUTER: Ipv4Addr = Ipv4Addr::new(11, 77, 0, 1);

fn ipv4(source: Ipv4Addr, destination: Ipv4Addr, body: &[u8]) -> Vec<u8> {
    let length = 20 + body.len();
    let mut bytes = vec![0x45, 0];
    bytes.extend_from_slice(&(length as u16).to_be_bytes());
    bytes.extend_from_slice(&[0, 1, 0x40, 0, 60, 1, 0, 0]);
    bytes.extend_from_slice(&source.octets());
    bytes.extend_from_slice(&destination.octets());
    let header = checksum(&bytes);
    bytes[10..12].copy_from_slice(&header.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

fn icmp(mut body: Vec<u8>) -> Vec<u8> {
    body[2..4].fill(0);
    let sum = checksum(&body);
    body[2..4].copy_from_slice(&sum.to_be_bytes());
    body
}

fn reply(request: &Packet, from: Ipv4Addr) -> Packet {
    let mut body = request.as_bytes()[20..].to_vec();
    body[0] = 0;
    Packet::parse(ipv4(from, SOURCE, &icmp(body))).unwrap()
}

fn error(request: &Packet, kind: u8, code: u8) -> Packet {
    let mut body = vec![kind, code, 0, 0, 0, 0, 0, 0];
    body.extend_from_slice(&request.as_bytes()[..28]);
    Packet::parse(ipv4(ROUTER, SOURCE, &icmp(body))).unwrap()
}

#[test]
fn a_request_is_a_valid_local_echo_packet_matched_only_by_its_own_reply() {
    let start = Instant::now();
    let mut echo = Echo::new(TARGET, 0x5157, start);
    assert!(echo.due(start));
    let request = echo.packet(SOURCE, start).unwrap();
    let facts = request.facts();
    assert_eq!(facts.traffic.source, IpAddr::V4(SOURCE));
    assert_eq!(facts.traffic.destination, IpAddr::V4(TARGET));
    assert!(echo.is_request(&facts));
    assert!(!echo.due(start + Duration::from_secs(2)), "one in flight");
    // An answer before admission is not measured.
    assert_eq!(echo.answer(&reply(&request, TARGET).facts(), start), None);
    echo.admitted(start, false);
    // Another source, a request (not a reply) and another sequence are not ours.
    assert_eq!(echo.answer(&reply(&request, ROUTER).facts(), start), None);
    assert_eq!(echo.answer(&facts, start), None);
    let at = start + Duration::from_millis(142);
    let result = echo.answer(&reply(&request, TARGET).facts(), at).unwrap();
    assert_eq!(
        result,
        Result {
            sequence: 0,
            outcome: Outcome::Reply,
            from: Some(IpAddr::V4(TARGET)),
            rtt: Some(Duration::from_millis(142)),
            setup: false,
        }
    );
    assert_eq!(
        echo.answer(&reply(&request, TARGET).facts(), at),
        None,
        "answered once"
    );
    assert!(!echo.due(at), "interval");
    assert!(echo.due(start + MIN_INTERVAL_FOR_TEST));
    let next = echo.packet(SOURCE, start + MIN_INTERVAL_FOR_TEST).unwrap();
    assert!(
        !echo.is_request(&facts),
        "the earlier sequence is no longer in flight"
    );
    assert!(echo.is_request(&next.facts()));
}

const MIN_INTERVAL_FOR_TEST: Duration = super::super::MIN_INTERVAL;

#[test]
fn an_icmp_error_quoting_the_request_maps_to_unreachable_from_its_sender() {
    let start = Instant::now();
    let mut echo = Echo::new(TARGET, 7, start);
    let request = echo.packet(SOURCE, start).unwrap();
    echo.admitted(start, true);
    let error = error(&request, 3, 1);
    assert!(matches!(
        error.transport(),
        Transport::Error(IcmpError {
            kind: 3,
            code: 1,
            ..
        })
    ));
    let result = echo
        .answer(&error.facts(), start + Duration::from_millis(30))
        .unwrap();
    assert_eq!(result.outcome, Outcome::Unreachable { kind: 3, code: 1 });
    assert_eq!(result.from, Some(IpAddr::V4(ROUTER)));
    assert!(result.setup, "the round trip includes the circuit setup");
    // Time exceeded quoting another identifier is someone else's.
    let mut other = Echo::new(TARGET, 8, start);
    let request = other.packet(SOURCE, start).unwrap();
    other.admitted(start, false);
    assert_eq!(
        echo.answer(&super::tests::error(&request, 11, 0).facts(), start),
        None
    );
}

#[test]
fn deadline_is_a_timeout_once_admitted_and_a_local_failure_otherwise() {
    let start = Instant::now();
    let mut echo = Echo::new(TARGET, 1, start);
    let request = echo.packet(SOURCE, start).unwrap();
    echo.admitted(start, false);
    assert_eq!(echo.expire(start + Duration::from_secs(1)), None);
    let late = start + super::super::DEADLINE + Duration::from_millis(1);
    assert_eq!(
        echo.answer(&reply(&request, TARGET).facts(), late),
        None,
        "late"
    );
    assert_eq!(echo.expire(late).unwrap().outcome, Outcome::Timeout);
    echo.packet(SOURCE, late).unwrap();
    let later = late + super::super::DEADLINE;
    assert_eq!(echo.expire(later).unwrap().outcome, Outcome::Failed);
    echo.packet(SOURCE, later).unwrap();
    let refused = echo.refused(later).unwrap();
    assert_eq!((refused.sequence, refused.outcome), (2, Outcome::Failed));
    assert_eq!(echo.expire(later + super::super::DEADLINE), None);
}
