//! Default-zone conntrack identity and exact deletion. Never flushes conntrack.
use super::{
    ExitError,
    command::{Commands, Tool},
    tuple::OriginalTuple,
};
use meshway_model::exit::ExitUnavailableReason as Reason;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tuple {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub protocol: u8,
    pub source_key: u16,
    pub destination_key: u16,
}

impl Tuple {
    pub fn key(&self) -> serde_json::Value {
        serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), self.protocol, self.source_key, self.destination_key]})
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetiredTuple {
    pub tuple: Tuple,
    pub sequence: u16,
}
impl RetiredTuple {
    pub fn key(&self) -> serde_json::Value {
        if self.tuple.protocol == 1 {
            serde_json::json!({"concat": [self.tuple.source.to_string(), self.tuple.destination.to_string(), self.tuple.source_key, self.sequence]})
        } else {
            self.tuple.key()
        }
    }
    pub fn set(&self) -> &'static str {
        if self.tuple.protocol == 1 {
            "echo_quarantine"
        } else {
            "quarantine"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub id: u32,
    pub mark: u32,
    pub original: Tuple,
    pub reply: Tuple,
}

pub(super) async fn lookup(
    commands: &mut Commands,
    tuple: OriginalTuple,
) -> Result<Option<Entry>, ExitError> {
    lookup_filtered(commands, tuple, None).await
}

/// An empty result is not a CT confirmation: conntrack is created by the
/// first packet, not by adding an nft qualification element.
pub(super) async fn lookup_owned(
    commands: &mut Commands,
    tuple: OriginalTuple,
    mark: u32,
) -> Result<Option<Entry>, ExitError> {
    if !mark.is_power_of_two() {
        return Err(Reason::OwnershipConflict.into());
    }
    let all = lookup_filtered(commands, tuple, None).await?;
    if all.as_ref().is_some_and(|entry| entry.mark & mark != mark) {
        return Err(Reason::OwnershipConflict.into());
    }
    let owned = lookup_filtered(commands, tuple, Some(mark)).await?;
    if owned
        .as_ref()
        .is_some_and(|entry| entry.mark & mark != mark)
        || (all.is_some() && owned != all)
    {
        return Err(Reason::OwnershipConflict.into());
    }
    Ok(owned)
}

async fn lookup_filtered(
    commands: &mut Commands,
    tuple: OriginalTuple,
    mark: Option<u32>,
) -> Result<Option<Entry>, ExitError> {
    let mut args = vec!["-L".into()];
    args.extend(tuple.filter());
    if let Some(mark) = mark {
        args.extend(["--mark".into(), format!("{mark}/{mark}")]);
    }
    args.extend([
        "--zone".into(),
        "0".into(),
        "-o".into(),
        "extended,id".into(),
    ]);
    let output = commands
        .run(Tool::Conntrack, args, Vec::new())
        .await
        .map_err(|_| Reason::ConntrackUnavailable)?;
    if !output.success {
        return Err(Reason::ConntrackUnavailable.into());
    }
    let entries = parse(&output.stdout)?;
    if entries.len() > 1 {
        return Err(Reason::OwnershipConflict.into());
    }
    let entry = entries.into_iter().next();
    if let Some(entry) = &entry {
        let (protocol, source_key, destination_key) = tuple.fields();
        if entry.original
            != (Tuple {
                source: tuple.source,
                destination: tuple.destination,
                protocol,
                source_key,
                destination_key,
            })
        {
            return Err(Reason::OwnershipConflict.into());
        }
    }
    Ok(entry)
}

pub(super) async fn delete(
    commands: &mut Commands,
    tuple: OriginalTuple,
    entry: &Entry,
    mark: u32,
) -> Result<(), ExitError> {
    if mark == 0 || !mark.is_power_of_two() || entry.mark & mark != mark {
        return Err(Reason::OwnershipConflict.into());
    }
    let current = lookup(commands, tuple).await?;
    let Some(current) = current else {
        return Ok(());
    };
    if current.id != entry.id
        || current.mark != entry.mark
        || current.original != entry.original
        || current.reply != entry.reply
    {
        return Err(Reason::OwnershipConflict.into());
    }
    let mut args = vec!["-D".into()];
    args.extend(tuple.filter());
    args.extend([
        "--zone".into(),
        "0".into(),
        "--mark".into(),
        format!("{mark}/{mark}"),
    ]);
    // The pinned conntrack-tools 1.4.8 crashes when --id is combined with -D.
    // Identity is checked above; deletion uses the required original tuple,
    // family, default zone and ownership mask, followed by an absence readback.
    // The gate must already be closed. A failed/lost delete ACK is settled and
    // inspected; absence, not the process status, proves retirement.
    let _outcome = commands.run(Tool::Conntrack, args, Vec::new()).await;
    #[cfg(test)]
    if let Ok(output) = &_outcome
        && !output.success
    {
        eprintln!(
            "conntrack delete status={}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if lookup(commands, tuple).await?.is_some() {
        return Err(Reason::ConntrackUnavailable.into());
    }
    Ok(())
}

pub(super) async fn mark_unused(commands: &mut Commands, mark: u32) -> Result<bool, ExitError> {
    let output = commands
        .run(
            Tool::Conntrack,
            vec![
                "-L".into(),
                "-f".into(),
                "ipv4".into(),
                "--mark".into(),
                format!("{mark}/{mark}"),
            ],
            Vec::new(),
        )
        .await
        .map_err(|_| Reason::ConntrackUnavailable)?;
    if !output.success {
        return Err(Reason::ConntrackUnavailable.into());
    }
    Ok(output.stdout.iter().all(u8::is_ascii_whitespace))
}

fn parse(bytes: &[u8]) -> Result<Vec<Entry>, ExitError> {
    let text = std::str::from_utf8(bytes).map_err(|_| Reason::ConntrackUnavailable)?;
    let mut result = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if result.len() == 2 || line.len() > 4096 {
            return Err(Reason::ConntrackUnavailable.into());
        }
        result.push(parse_line(line).ok_or(Reason::ConntrackUnavailable)?);
    }
    Ok(result)
}

fn parse_line(line: &str) -> Option<Entry> {
    let fields: Vec<_> = line.split_ascii_whitespace().collect();
    let mut offset = 0;
    if fields.get(offset) == Some(&"ipv4") {
        if fields.get(1) != Some(&"2") {
            return None;
        }
        offset = 2;
    }
    let protocol = match *fields.get(offset)? {
        "tcp" => 6,
        "udp" => 17,
        "icmp" => 1,
        _ => return None,
    };
    if fields.get(offset + 1)?.parse::<u8>().ok()? != protocol {
        return None;
    }
    fields.get(offset + 2)?.parse::<u32>().ok()?;
    offset += 3;
    let mut groups: Vec<Vec<(&str, &str)>> = Vec::new();
    let mut id = None;
    let mut mark = None;
    for field in &fields[offset..] {
        if matches!(
            *field,
            "SYN_SENT"
                | "SYN_RECV"
                | "ESTABLISHED"
                | "FIN_WAIT"
                | "CLOSE_WAIT"
                | "LAST_ACK"
                | "TIME_WAIT"
                | "CLOSE"
                | "LISTEN"
                | "NONE"
                | "[UNREPLIED]"
                | "[ASSURED]"
        ) {
            continue;
        }
        let (key, value) = field.split_once('=')?;
        match key {
            "id" if groups.len() == 2
                && groups[1].iter().any(|(key, _)| *key == "dst")
                && (protocol != 1 || groups[1].iter().any(|(key, _)| *key == "id")) =>
            {
                if id.replace(value.parse::<u32>().ok()?).is_some() {
                    return None;
                }
            }
            "mark" => {
                if mark.replace(value.parse::<u32>().ok()?).is_some() {
                    return None;
                }
            }
            "zone" => {
                if value != "0" {
                    return None;
                }
            }
            "use" | "packets" | "bytes" => {
                value.parse::<u64>().ok()?;
            }
            "src" => {
                if groups.len() == 2 {
                    return None;
                }
                groups.push(vec![(key, value)]);
            }
            "dst" | "sport" | "dport" | "type" | "code" | "id" => {
                groups.last_mut()?.push((key, value));
            }
            _ => return None,
        }
    }
    if groups.len() != 2 {
        return None;
    }
    let tuple = |group: &[(&str, &str)]| -> Option<Tuple> {
        let value = |wanted| {
            let mut found = group.iter().filter(|(key, _)| *key == wanted);
            let value = found.next()?.1;
            if found.next().is_some() {
                return None;
            }
            Some(value)
        };
        let (source_key, destination_key) = if protocol == 1 {
            if group.len() != 5 {
                return None;
            }
            let kind = value("type")?.parse::<u8>().ok()?;
            let code = value("code")?.parse::<u8>().ok()?;
            if !matches!(kind, 0 | 8) || code != 0 {
                return None;
            }
            (value("id")?.parse().ok()?, u16::from(kind) << 8)
        } else {
            if group.len() != 4 {
                return None;
            }
            (value("sport")?.parse().ok()?, value("dport")?.parse().ok()?)
        };
        Some(Tuple {
            source: value("src")?.parse().ok()?,
            destination: value("dst")?.parse().ok()?,
            protocol,
            source_key,
            destination_key,
        })
    };
    Some(Entry {
        id: id?,
        mark: mark?,
        original: tuple(&groups[0])?,
        reply: tuple(&groups[1])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_conntrack_identity_rejects_unknown_zone_offload_and_duplicate_fields() {
        let line = "ipv4 2 udp 17 29 src=100.96.0.1 dst=192.0.2.2 sport=12345 dport=53 [UNREPLIED] src=192.0.2.2 dst=198.51.100.1 sport=53 dport=23456 mark=8 use=1 id=123";
        let entry = parse_line(line).unwrap();
        assert_eq!(entry.id, 123);
        assert_eq!(entry.reply.destination_key, 23456);
        for suffix in [" zone=1", " [OFFLOAD]", " mark=8", " id=124", " mystery=1"] {
            assert!(parse_line(&format!("{line}{suffix}")).is_none());
        }
        let echo = "ipv4 2 icmp 1 29 src=100.96.0.1 dst=192.0.2.2 type=8 code=0 id=321 src=192.0.2.2 dst=198.51.100.1 type=0 code=0 id=456 mark=8 use=1 id=987";
        let echo = parse_line(echo).unwrap();
        assert_eq!(echo.id, 987);
        assert_eq!(echo.original.source_key, 321);
        assert_eq!(echo.reply.source_key, 456);
    }
}
