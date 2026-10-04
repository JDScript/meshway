//! A minimal MaxMind DB writer for tests: IPv4-only tree, 24-bit records, one
//! data record per prefix. Enough for the reader to decode City and ASN
//! records exactly as a real GeoLite2 file would present them.
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

#[derive(Clone)]
pub enum Value {
    Str(String),
    U16(u16),
    U32(u32),
    U64(u64),
    F64(f64),
    Map(Vec<(String, Value)>),
    Array(Vec<Value>),
}

pub fn city(country: &str, city: &str, latitude: f64, longitude: f64) -> Value {
    Value::Map(vec![
        (
            "country".into(),
            Value::Map(vec![("iso_code".into(), Value::Str(country.into()))]),
        ),
        (
            "city".into(),
            Value::Map(vec![(
                "names".into(),
                Value::Map(vec![("en".into(), Value::Str(city.into()))]),
            )]),
        ),
        (
            "location".into(),
            Value::Map(vec![
                ("latitude".into(), Value::F64(latitude)),
                ("longitude".into(), Value::F64(longitude)),
            ]),
        ),
    ])
}

pub fn asn(number: u32, organization: &str) -> Value {
    Value::Map(vec![
        ("autonomous_system_number".into(), Value::U32(number)),
        (
            "autonomous_system_organization".into(),
            Value::Str(organization.into()),
        ),
    ])
}

/// `database_type` is what the Server checks ("GeoLite2-City", "GeoLite2-ASN").
pub fn database(database_type: &str, entries: &[(Ipv4Addr, u8, Value)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for (_, _, value) in entries {
        offsets.push(data.len());
        encode(value, &mut data);
    }
    // Tree: node 0 is the root; records point at nodes, at `node_count` for
    // "no data", or at node_count + 16 + data offset.
    let mut nodes: Vec<[Option<Record>; 2]> = vec![[None, None]];
    for ((address, length, _), offset) in entries.iter().zip(&offsets) {
        let bits = u32::from(*address);
        let mut node = 0usize;
        for depth in 0..*length {
            let bit = ((bits >> (31 - depth)) & 1) as usize;
            if depth + 1 == *length {
                nodes[node][bit] = Some(Record::Data(*offset));
            } else {
                let next = match nodes[node][bit] {
                    Some(Record::Node(next)) => next,
                    None => {
                        nodes.push([None, None]);
                        let next = nodes.len() - 1;
                        nodes[node][bit] = Some(Record::Node(next));
                        next
                    }
                    Some(Record::Data(_)) => panic!("overlapping prefixes"),
                };
                node = next;
            }
        }
    }
    let node_count = nodes.len();
    let mut out = Vec::new();
    for node in &nodes {
        for record in node {
            let value = match record {
                None => node_count,
                Some(Record::Node(next)) => *next,
                Some(Record::Data(offset)) => node_count + 16 + offset,
            } as u32;
            out.extend_from_slice(&value.to_be_bytes()[1..]);
        }
    }
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\xab\xcd\xefMaxMind.com");
    let mut languages = BTreeMap::new();
    languages.insert("en", "test");
    encode(
        &Value::Map(vec![
            ("binary_format_major_version".into(), Value::U16(2)),
            ("binary_format_minor_version".into(), Value::U16(0)),
            ("build_epoch".into(), Value::U64(1_700_000_000)),
            ("database_type".into(), Value::Str(database_type.into())),
            (
                "description".into(),
                Value::Map(vec![("en".into(), Value::Str("test dataset".into()))]),
            ),
            ("ip_version".into(), Value::U16(4)),
            (
                "languages".into(),
                Value::Array(vec![Value::Str("en".into())]),
            ),
            ("node_count".into(), Value::U32(node_count as u32)),
            ("record_size".into(), Value::U16(24)),
        ]),
        &mut out,
    );
    out
}

#[derive(Clone, Copy)]
enum Record {
    Node(usize),
    Data(usize),
}

fn control(kind: u8, size: usize, out: &mut Vec<u8>) {
    // Types 1..=7 fit the control byte; larger types use the extended form.
    let (first, extended) = if kind <= 7 {
        (kind << 5, None)
    } else {
        (0, Some(kind - 7))
    };
    // Sizes below 29 fit the control byte; 29..=284 take one extra byte.
    assert!(size <= 284, "test values stay small");
    if size < 29 {
        out.push(first | size as u8);
    } else {
        out.push(first | 29);
    }
    if let Some(extended) = extended {
        out.push(extended);
    }
    if size >= 29 {
        out.push((size - 29) as u8);
    }
}

fn encode(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Str(text) => {
            control(2, text.len(), out);
            out.extend_from_slice(text.as_bytes());
        }
        Value::U16(number) => integer(5, &number.to_be_bytes(), out),
        Value::U32(number) => integer(6, &number.to_be_bytes(), out),
        Value::U64(number) => integer(9, &number.to_be_bytes(), out),
        Value::F64(number) => {
            control(3, 8, out);
            out.extend_from_slice(&number.to_be_bytes());
        }
        Value::Map(entries) => {
            control(7, entries.len(), out);
            for (key, value) in entries {
                encode(&Value::Str(key.clone()), out);
                encode(value, out);
            }
        }
        Value::Array(items) => {
            control(11, items.len(), out);
            for item in items {
                encode(item, out);
            }
        }
    }
}

fn integer(kind: u8, bytes: &[u8], out: &mut Vec<u8>) {
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    control(kind, bytes.len() - start, out);
    out.extend_from_slice(&bytes[start..]);
}
