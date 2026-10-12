//! Trusted DNS associations at both ends of a Mesh DNS service.
//!
//! Exit resolver: after answering a source's query whose name a name rule of
//! that source decides, it completes the standing association task the
//! served plan issued ([`crate::OwnerDispatcher::with_association_reports`]);
//! the answer itself goes back to the source at once.
//!
//! Source: such an answer reaches the local client only once Engine applied
//! an association of the same name covering its public addresses, at most
//! [`ASSOCIATION_WAIT`] later (otherwise SERVFAIL, so no client ever holds
//! an address its name rule could not decide). Its TTLs are capped to the
//! association's remaining life, so caches end first.
use crate::{Query, wire::WireError};
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::{RData, RecordType},
};
use meshway_configuration::{
    execution::ExecutionAddress,
    netmap::{
        AssociationRecordType, MAX_ASSOCIATION_ADDRESSES, MAX_ASSOCIATION_CNAMES,
        MAX_ASSOCIATION_LIFETIME_MS, MIN_ASSOCIATION_LIFETIME_MS,
    },
};
use meshway_policy::compile::{netmap::associable, rules::query_name};
use std::{net::IpAddr, time::Duration};

/// Longest a source holds an answer for its association.
pub const ASSOCIATION_WAIT: Duration = Duration::from_secs(2);
/// An association with less life left is waited for anew.
pub(crate) const MIN_REMAINING_MS: u64 = 5_000;
/// Margin between a capped TTL and the association's end.
const TTL_MARGIN_SECONDS: u64 = 2;
/// Exit reports pending at most; a full queue drops the newest.
pub const MAX_PENDING_REPORTS: usize = 256;

/// The association-relevant content of one NOERROR answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answered {
    pub name: String,
    pub record_type: AssociationRecordType,
    pub cnames: Vec<String>,
    /// Public addresses of the chain's final owner, sorted, unique, bounded.
    pub addresses: Vec<IpAddr>,
    /// Shortest TTL of the used chain and address records.
    pub ttl_seconds: u32,
}

impl Answered {
    pub fn lifetime_ms(&self) -> u64 {
        (u64::from(self.ttl_seconds) * 1000)
            .clamp(MIN_ASSOCIATION_LIFETIME_MS, MAX_ASSOCIATION_LIFETIME_MS)
    }
}

fn name_of(name: &hickory_proto::rr::Name) -> String {
    query_name(&name.to_ascii())
}

/// What `message` answers for `query`; `None` unless it is a NOERROR answer
/// to an A or AAAA question with at least one public address.
pub(crate) fn answered(query: &Query, message: &Message) -> Option<Answered> {
    let question = query.question();
    let record_type = match question.query_type() {
        RecordType::A => AssociationRecordType::A,
        RecordType::AAAA => AssociationRecordType::Aaaa,
        _ => return None,
    };
    if message.metadata.response_code != ResponseCode::NoError {
        return None;
    }
    let name = name_of(question.name());
    let mut owner = name.clone();
    let mut cnames = Vec::new();
    let mut ttl = u32::MAX;
    let mut addresses = Vec::new();
    // A chain only follows records in order of their owners; at most eight hops.
    for _ in 0..=MAX_ASSOCIATION_CNAMES {
        let mut next = None;
        for record in &message.answers {
            if name_of(&record.name) != owner {
                continue;
            }
            match &record.data {
                RData::CNAME(target) if next.is_none() => {
                    next = Some(name_of(&target.0));
                    ttl = ttl.min(record.ttl);
                }
                RData::A(address) if record_type == AssociationRecordType::A => {
                    addresses.push(IpAddr::V4(address.0));
                    ttl = ttl.min(record.ttl);
                }
                RData::AAAA(address) if record_type == AssociationRecordType::Aaaa => {
                    addresses.push(IpAddr::V6(address.0));
                    ttl = ttl.min(record.ttl);
                }
                _ => {}
            }
        }
        match next {
            Some(target) if addresses.is_empty() => {
                if cnames.len() == MAX_ASSOCIATION_CNAMES
                    || cnames.contains(&target)
                    || target == name
                {
                    return None;
                }
                cnames.push(target.clone());
                owner = target;
            }
            _ => break,
        }
    }
    addresses.retain(|address| associable(*address));
    addresses.sort();
    addresses.dedup();
    addresses.truncate(MAX_ASSOCIATION_ADDRESSES);
    if addresses.is_empty() || name.is_empty() {
        return None;
    }
    Some(Answered {
        name,
        record_type,
        cnames,
        addresses,
        ttl_seconds: ttl,
    })
}

/// Cap every record TTL of `message` to the remaining life of its association.
pub(crate) fn cap_ttl(message: &mut Message, remaining_ms: u64) {
    let cap = u32::try_from(
        (remaining_ms / 1000)
            .saturating_sub(TTL_MARGIN_SECONDS)
            .max(1),
    )
    .unwrap_or(u32::MAX);
    for record in message
        .answers
        .iter_mut()
        .chain(message.authorities.iter_mut())
    {
        record.ttl = record.ttl.min(cap);
    }
}

pub(crate) fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// A single-use report identifier.
pub(crate) fn report_id() -> Result<meshway_model::RequestId, WireError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| WireError::QueryShape)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
    .parse()
    .map_err(|_| WireError::QueryShape)
}

/// The completion an exit sends for `answered`.
pub(crate) fn report(
    netmap: meshway_model::NetmapRef,
    service: &meshway_configuration::netmap::DnsService,
    answered: &Answered,
) -> Result<meshway_protocol::DnsAssociationReport, WireError> {
    Ok(meshway_protocol::DnsAssociationReport {
        netmap,
        report_id: report_id()?,
        source: service.source,
        generation: service.generation,
        name: answered.name.clone(),
        record_type: answered.record_type,
        cnames: answered.cnames.clone(),
        addresses: answered
            .addresses
            .iter()
            .copied()
            .map(ExecutionAddress)
            .collect(),
        ttl_seconds: answered.ttl_seconds,
        resolved_at_ms: wall_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::{
        op::{Message, MessageType, OpCode, Query as Question},
        rr::{Name, Record, rdata},
    };
    use std::str::FromStr;

    fn query(name: &str, kind: RecordType) -> (Query, Message) {
        let mut message = Message::query();
        message.add_query(Question::query(Name::from_str(name).unwrap(), kind));
        message.metadata.recursion_desired = true;
        let bytes = message.to_vec().unwrap();
        let query = crate::decode_query(&bytes).unwrap();
        let mut response = query.response();
        response.metadata.message_type = MessageType::Response;
        response.metadata.op_code = OpCode::Query;
        (query, response)
    }

    #[test]
    fn follows_the_cname_chain_and_keeps_public_addresses_only() {
        let (query, mut message) = query("Chat.Example.", RecordType::A);
        let owner = Name::from_str("chat.example.").unwrap();
        let edge = Name::from_str("edge.cdn.example.").unwrap();
        message.add_answer(Record::from_rdata(
            owner,
            300,
            RData::CNAME(rdata::CNAME(edge.clone())),
        ));
        for (address, ttl) in [("203.0.113.9", 60), ("10.0.0.1", 60), ("8.8.4.4", 40)] {
            message.add_answer(Record::from_rdata(
                edge.clone(),
                ttl,
                RData::A(rdata::A(address.parse().unwrap())),
            ));
        }
        let facts = answered(&query, &message).unwrap();
        assert_eq!(facts.name, "chat.example");
        assert_eq!(facts.cnames, ["edge.cdn.example"]);
        assert_eq!(facts.addresses, ["8.8.4.4".parse::<IpAddr>().unwrap()]);
        assert_eq!(facts.ttl_seconds, 40);
        assert_eq!(facts.lifetime_ms(), 40_000);
        cap_ttl(&mut message, 9_000);
        assert!(message.answers.iter().all(|record| record.ttl <= 7));
    }

    #[test]
    fn private_only_errors_and_loops_produce_nothing() {
        let (query, mut message) = query("lan.example.", RecordType::A);
        let owner = Name::from_str("lan.example.").unwrap();
        message.add_answer(Record::from_rdata(
            owner.clone(),
            60,
            RData::A(rdata::A("192.168.1.2".parse().unwrap())),
        ));
        assert_eq!(answered(&query, &message), None);
        let (query, mut looped) = super::tests::query("a.example.", RecordType::A);
        let a = Name::from_str("a.example.").unwrap();
        let b = Name::from_str("b.example.").unwrap();
        looped.add_answer(Record::from_rdata(
            a.clone(),
            60,
            RData::CNAME(rdata::CNAME(b.clone())),
        ));
        looped.add_answer(Record::from_rdata(b, 60, RData::CNAME(rdata::CNAME(a))));
        assert_eq!(answered(&query, &looped), None);
        let (query, mut failed) = super::tests::query("lan.example.", RecordType::A);
        failed.metadata.response_code = ResponseCode::ServFail;
        assert_eq!(answered(&query, &failed), None);
    }
}
