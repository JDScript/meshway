//! Canonical ownership receipts from nft echo, checked against subsequent dumps.
use super::{
    command::{Commands, Tool},
    program::Program,
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::linux::kernel) struct Receipt {
    /// Object key includes the rule's order inside its chain.
    pub objects: BTreeMap<String, Value>,
    pub handles: BTreeMap<String, u64>,
    pub elements: BTreeMap<String, Vec<Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    nftables: Vec<Value>,
}

pub(in crate::linux::kernel) async fn mutate(
    commands: &mut Commands,
    batch: &Value,
) -> Result<Receipt> {
    let output = commands
        .run(
            Tool::Nft,
            vec![
                "-j".into(),
                "-nnn".into(),
                "-e".into(),
                "-f".into(),
                "-".into(),
            ],
            serde_json::to_vec(batch).map_err(|_| Error::Journal)?,
        )
        .await
        .map_err(|_| Error::Netlink)?;
    if !output.success {
        #[cfg(test)]
        eprintln!("nft mutation: {}", String::from_utf8_lossy(&output.stderr));
        return Err(Error::Netlink);
    }
    let decoded = decode(&output.stdout);
    #[cfg(test)]
    if decoded.is_err() {
        eprintln!(
            "nft echo decode: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    decoded
}

pub(super) async fn dump(commands: &mut Commands, program: &Program) -> Result<Receipt> {
    let mut all = Receipt {
        objects: BTreeMap::new(),
        handles: BTreeMap::new(),
        elements: BTreeMap::new(),
    };
    for (family, name) in program.tables() {
        let output = commands
            .run(
                Tool::Nft,
                vec![
                    "-j".into(),
                    "-nnn".into(),
                    "list".into(),
                    "table".into(),
                    family.into(),
                    name,
                ],
                Vec::new(),
            )
            .await
            .map_err(|_| Error::Netlink)?;
        if !output.success {
            return Err(Error::Conflict);
        }
        let mut receipt = decode(&output.stdout)?;
        for value in receipt.objects.values_mut() {
            normalize(value, Some(program))?;
        }
        for (key, value) in receipt.objects {
            if all.objects.insert(key, value).is_some() {
                return Err(Error::Conflict);
            }
        }
        all.handles.extend(receipt.handles);
        all.elements.extend(receipt.elements);
    }
    Ok(all)
}

pub(super) async fn present(commands: &mut Commands, program: &Program) -> Result<usize> {
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "tables".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| Error::Netlink)?;
    if !output.success {
        return Err(Error::Netlink);
    }
    let document: Document = serde_json::from_slice(&output.stdout).map_err(|_| Error::Journal)?;
    if document.nftables.len() > 16384 {
        return Err(Error::Capacity);
    }
    let mut found = std::collections::BTreeSet::new();
    for object in document.nftables {
        if object.get("metainfo").is_some() {
            continue;
        }
        let table = object.get("table").ok_or(Error::Journal)?;
        let family = table
            .get("family")
            .and_then(Value::as_str)
            .ok_or(Error::Journal)?;
        let name = table
            .get("name")
            .and_then(Value::as_str)
            .ok_or(Error::Journal)?;
        if program
            .tables()
            .iter()
            .any(|(expected_family, expected_name)| {
                family == *expected_family && name == expected_name
            })
            && !found.insert((family.to_owned(), name.to_owned()))
        {
            return Err(Error::Conflict);
        }
    }
    Ok(found.len())
}

/// Reconcile the atomic CREATE whose echo could have been lost. The random
/// names, table comments and complete canonical program were durable before it
/// ran. No flow mutation can precede the first receipt.
pub(super) async fn recover_create(commands: &mut Commands, program: &Program) -> Result<Receipt> {
    let intended = decode(&serde_json::to_vec(&program.create()).map_err(|_| Error::Journal)?)?;
    let observed = dump(commands, program).await?;
    if observed.objects != intended.objects
        || observed.handles.len() != intended.objects.len()
        || observed
            .elements
            .values()
            .any(|elements| !elements.is_empty())
    {
        return Err(Error::Conflict);
    }
    Ok(observed)
}

pub(in crate::linux::kernel) fn decode(bytes: &[u8]) -> Result<Receipt> {
    let doc: Document = serde_json::from_slice(bytes).map_err(|_| Error::Journal)?;
    if doc.nftables.len() > 16384 {
        return Err(Error::Capacity);
    }
    let mut result = Receipt {
        objects: BTreeMap::new(),
        handles: BTreeMap::new(),
        elements: BTreeMap::new(),
    };
    let mut positions = BTreeMap::<String, u32>::new();
    for mut item in doc.nftables {
        if item.get("metainfo").is_some() {
            continue;
        }
        if item.get("delete").is_some() || item.get("flush").is_some() {
            continue;
        }
        for op in ["add", "create", "insert", "replace"] {
            if let Some(inner) = item.get(op) {
                item = inner.clone();
                break;
            }
        }
        let object = item.as_object_mut().ok_or(Error::Journal)?;
        // nft 1.0.9 places a table's echoed handle beside the table object;
        // list output and the other object echoes put it inside the object.
        if let Some(handle) = object.remove("handle") {
            if object.len() != 1 || handle.as_u64().filter(|v| *v != 0).is_none() {
                return Err(Error::Journal);
            }
            let data = object
                .get_mut("table")
                .and_then(Value::as_object_mut)
                .ok_or(Error::Journal)?;
            if data.insert("handle".into(), handle).is_some() {
                return Err(Error::Journal);
            }
        }
        if object.len() != 1 {
            return Err(Error::Journal);
        }
        let (kind, data) = object.iter_mut().next().ok_or(Error::Journal)?;
        // Element echoes carry no new static object identity.
        if kind == "element" {
            continue;
        }
        let allowed: &[&str] = match kind.as_str() {
            "table" => &["family", "name", "handle", "comment", "flags"],
            "chain" => &[
                "family", "table", "name", "handle", "type", "hook", "prio", "policy", "comment",
                "flags",
            ],
            "set" | "map" => &[
                "family",
                "table",
                "name",
                "handle",
                "type",
                "map",
                "flags",
                "timeout",
                "gc-interval",
                "size",
                "elem",
                "policy",
                "comment",
                "auto-merge",
                "count",
            ],
            "rule" => &[
                "family", "table", "chain", "handle", "expr", "comment", "index",
            ],
            _ => return Err(Error::Journal),
        };
        let data = data.as_object_mut().ok_or(Error::Journal)?;
        if data.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(Error::Journal);
        }
        let text = |key: &str| {
            data.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(Error::Journal)
        };
        let family = text("family")?;
        if !matches!(family.as_str(), "inet" | "ip") {
            return Err(Error::Journal);
        }
        let table = if kind == "table" {
            text("name")?
        } else {
            text("table")?
        };
        let suffix = if kind == "rule" {
            let chain = text("chain")?;
            let position = positions
                .entry(format!("{family}/{table}/{chain}"))
                .or_default();
            let suffix = format!("{chain}/{position:05}");
            *position += 1;
            suffix
        } else {
            text("name")?
        };
        let key = format!("{family}/{table}/{kind}/{suffix}");
        if let Some(handle) = data.remove("handle") {
            let handle = handle.as_u64().filter(|v| *v != 0).ok_or(Error::Journal)?;
            result.handles.insert(key.clone(), handle);
        }
        data.remove("index");
        if matches!(kind.as_str(), "set" | "map") {
            data.remove("count");
            // Permission data is checked separately from immutable object shape.
            if !matches!(data.get("name").and_then(Value::as_str), Some("mesh"))
                && let Some(elements) = data.remove("elem")
            {
                let elements = elements
                    .as_array()
                    .filter(|elements| elements.len() <= super::program::HISTORY)
                    .ok_or(Error::Capacity)?
                    .clone();
                result.elements.insert(key.clone(), elements);
            }
            data.entry("flags").or_insert(json!([]));
            data.entry("policy").or_insert(json!("performance"));
            data.entry("timeout").or_insert(json!(0));
            data.entry("gc-interval").or_insert(json!(0));
            data.entry("auto-merge").or_insert(json!(false));
        }
        let mut value = Value::Object(data.clone());
        normalize(&mut value, None)?;
        if result.objects.insert(key, value).is_some() {
            return Err(Error::Journal);
        }
    }
    Ok(result)
}

fn normalize(value: &mut Value, program: Option<&Program>) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize(value, program)?;
            }
        }
        Value::Object(values) => {
            if values.get("payload").is_some_and(|payload| {
                payload.get("protocol").and_then(Value::as_str) == Some("icmp")
                    && payload.get("field").and_then(Value::as_str) == Some("gateway")
            }) {
                // nft delinearizes the 32-bit word at ICMP offset 4 as the
                // union's gateway member, even when the rule admits only echo.
                values.insert(
                    "payload".into(),
                    json!({"base": "th", "offset": 32, "len": 32}),
                );
            }
            for value in values.values_mut() {
                normalize(value, program)?;
            }
            if let Some(ct) = values.get_mut("ct").and_then(Value::as_object_mut)
                && ct.get("family").and_then(Value::as_str) == Some("ip")
                && let Some(key) = ct
                    .get("key")
                    .and_then(Value::as_str)
                    .filter(|key| matches!(*key, "saddr" | "daddr"))
                    .map(str::to_owned)
            {
                ct.insert("key".into(), json!(format!("ip {key}")));
                ct.remove("family");
            }
            if let Some(compare) = values.get_mut("match").and_then(Value::as_object_mut) {
                normalize_mark_prefix(compare)?;
                let left = compare.get("left").cloned().ok_or(Error::Journal)?;
                let right = compare.get_mut("right").ok_or(Error::Journal)?;
                if let Some(key) = left
                    .get("meta")
                    .and_then(|meta| meta.get("key"))
                    .and_then(Value::as_str)
                {
                    match (key, right.as_str()) {
                        ("nfproto", Some("ipv4")) => *right = json!(2),
                        ("nfproto", Some("ipv6")) => *right = json!(10),
                        ("iif" | "oif", Some(name)) => {
                            if let Some(program) = program {
                                if name != program.interface_name
                                    && name != program.interface.to_string()
                                {
                                    #[cfg(test)]
                                    eprintln!(
                                        "nft interface mismatch {name:?}, wanted {} or {}",
                                        program.interface_name, program.interface
                                    );
                                    return Err(Error::Conflict);
                                }
                                *right = json!(program.interface);
                            }
                        }
                        _ => {}
                    }
                }
                if left
                    .get("fib")
                    .and_then(|fib| fib.get("result"))
                    .and_then(Value::as_str)
                    == Some("type")
                {
                    let convert = |value: &mut Value| -> Result<()> {
                        if let Some(name) = value.as_str() {
                            *value = json!(match name {
                                "local" => 2,
                                "broadcast" => 3,
                                "multicast" => 5,
                                _ => return Err(Error::Journal),
                            });
                        }
                        Ok(())
                    };
                    if let Some(values) = right.get_mut("set").and_then(Value::as_array_mut) {
                        for value in values.iter_mut() {
                            convert(value)?;
                        }
                        values.sort_by_key(Value::as_u64);
                    } else {
                        convert(right)?;
                    }
                }
                if left
                    .get("ct")
                    .and_then(|ct| ct.get("key"))
                    .and_then(Value::as_str)
                    == Some("direction")
                {
                    match right.as_str() {
                        Some("original") => *right = json!(0),
                        Some("reply") => *right = json!(1),
                        _ => {}
                    }
                }
                if left
                    .get("ct")
                    .and_then(|ct| ct.get("key"))
                    .and_then(Value::as_str)
                    == Some("state")
                {
                    let mut states = right
                        .get("set")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_else(|| vec![right.clone()]);
                    for state in &mut states {
                        if let Some(name) = state.as_str() {
                            *state = json!(match name {
                                "invalid" => 1,
                                "established" => 2,
                                "related" => 4,
                                "new" => 8,
                                "untracked" => 64,
                                _ => return Err(Error::Journal),
                            });
                        }
                    }
                    states.sort_by_key(Value::as_u64);
                    *right = json!({"set": states});
                    if compare.get("op").and_then(Value::as_str) == Some("in") {
                        compare.insert("op".into(), json!("=="));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// nft delinearizes contiguous high mark masks as numeric prefixes. Convert
/// only exact 32-bit mark equality/inequality forms, preserving the full mask;
/// IP prefixes and other expressions remain subject to ordinary exact auditing.
fn normalize_mark_prefix(compare: &mut serde_json::Map<String, Value>) -> Result<()> {
    if !matches!(compare.get("op").and_then(Value::as_str), Some("==" | "!=")) {
        return Ok(());
    }
    let Some(left) = compare
        .get("left")
        .filter(|left| {
            **left == json!({"meta": {"key": "mark"}}) || **left == json!({"ct": {"key": "mark"}})
        })
        .cloned()
    else {
        return Ok(());
    };
    let Some(right) = compare
        .get("right")
        .and_then(Value::as_object)
        .filter(|right| right.len() == 1)
    else {
        return Ok(());
    };
    let Some(prefix) = right.get("prefix").and_then(Value::as_object) else {
        return Ok(());
    };
    if prefix.len() != 2 {
        return Err(Error::Journal);
    }
    let address = prefix
        .get("addr")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(Error::Journal)?;
    let length = prefix
        .get("len")
        .and_then(Value::as_u64)
        .filter(|length| *length <= 32)
        .ok_or(Error::Journal)? as u32;
    let mask = u32::MAX.checked_shl(32 - length).unwrap_or(0);
    if address & mask != address {
        return Err(Error::Journal);
    }
    compare.insert("left".into(), json!({"&": [left, mask]}));
    compare.insert("right".into(), json!(address));
    Ok(())
}

pub(super) async fn verify(
    commands: &mut Commands,
    program: &Program,
    expected: &Receipt,
) -> Result<()> {
    let observed = dump(commands, program).await?;
    if observed.objects != expected.objects || observed.handles != expected.handles {
        #[cfg(test)]
        {
            for (key, value) in &observed.objects {
                if expected.objects.get(key) != Some(value) {
                    eprintln!(
                        "nft shape mismatch {key}: expected={:?} actual={value}",
                        expected.objects.get(key)
                    );
                    break;
                }
            }
            if observed.handles != expected.handles {
                eprintln!(
                    "nft handle mismatch expected={:?} actual={:?}",
                    expected.handles, observed.handles
                );
            }
        }
        return Err(Error::Conflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires isolated Linux NFT/TUN namespace and NET_ADMIN"]
    async fn exit_nft_high_mark_prefix_preserves_exact_program_ownership() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        let tun = meshway_tun::Tun::create(meshway_tun::Options::new(1280).unwrap()).unwrap();
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let program = Program {
            legacy: false,
            installation: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
            interface: tun.if_index().unwrap(),
            interface_name: tun.name().to_owned(),
            mark: 0x8000_0000,
            pools: vec!["100.96.0.0/11".parse().unwrap()],
        };
        let mut commands = Commands::default();
        let receipt = mutate(&mut commands, &program.create()).await.unwrap();
        let result = verify(&mut commands, &program, &receipt).await;
        mutate(&mut commands, &json!({"nftables": program.tables().into_iter().map(|(family, name)| json!({"delete": {"table": {"family": family, "name": name}}})).collect::<Vec<_>>()})).await.unwrap();
        assert_eq!(present(&mut commands, &program).await.unwrap(), 0);
        assert!(
            result.is_ok(),
            "high-bit mark must survive actual NFT readback: {result:?}"
        );
    }

    fn rule(expression: Value) -> Value {
        json!({"nftables": [{"rule": {"family": "inet", "table": "owned", "chain": "output", "expr": [expression]}}]})
    }

    #[test]
    fn nft_high_mark_prefix_readback_matches_the_owned_bit_test() {
        for key in [
            json!({"meta": {"key": "mark"}}),
            json!({"ct": {"key": "mark"}}),
        ] {
            for op in ["==", "!="] {
                for address in [0, 0x8000_0000u32] {
                    let intended = rule(
                        json!({"match": {"left": {"&": [key, 0x8000_0000u32]}, "op": op, "right": address}}),
                    );
                    let observed = rule(
                        json!({"match": {"left": key, "op": op, "right": {"prefix": {"addr": address, "len": 1}}}}),
                    );
                    assert_eq!(
                        decode(&serde_json::to_vec(&intended).unwrap()).unwrap(),
                        decode(&serde_json::to_vec(&observed).unwrap()).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn mark_prefix_normalization_does_not_hide_changed_ownership_bits() {
        let intended = rule(
            json!({"match": {"left": {"&": [{"meta": {"key": "mark"}}, 0x8000_0000u32]}, "op": "!=", "right": 0}}),
        );
        let changed = rule(
            json!({"match": {"left": {"meta": {"key": "mark"}}, "op": "!=", "right": {"prefix": {"addr": 0, "len": 2}}}}),
        );
        assert_ne!(
            decode(&serde_json::to_vec(&intended).unwrap()).unwrap(),
            decode(&serde_json::to_vec(&changed).unwrap()).unwrap()
        );
    }
}
