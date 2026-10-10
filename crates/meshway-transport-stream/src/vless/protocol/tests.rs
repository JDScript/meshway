use super::*;
use tokio::io::AsyncWriteExt;

const UUID: &str = "00112233-4455-6677-8899-aabbccddeeff";

fn endpoint(host: &str) -> Endpoint {
    Endpoint {
        host: host.into(),
        port: Port::new(443).unwrap(),
    }
}

fn fixture(address: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
        0xee, 0xff, 0, 1, 1, 0xbb,
    ];
    bytes.extend_from_slice(address);
    bytes
}

#[test]
fn strict_hyphenated_uuid_accepts_uppercase_only_as_equivalent_encoding() {
    assert_eq!(
        *uuid(UUID).unwrap(),
        *uuid(&UUID.to_ascii_uppercase()).unwrap()
    );
    for invalid in [
        UUID.replace('-', ""),
        format!("{{{UUID}}}"),
        format!("urn:uuid:{UUID}"),
        UUID.replace('a', "g"),
    ] {
        assert!(matches!(uuid(&invalid), Err(Error::Configuration)));
    }
}

#[tokio::test]
async fn reference_wire_order_ipv4_ipv6_and_domain_with_payload_left_unread() {
    let key = uuid(UUID).unwrap();
    for (host, address) in [
        ("192.0.2.1", vec![1, 192, 0, 2, 1]),
        (
            "2001:db8::1",
            vec![3, 0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        ),
        ("example.com", b"\x02\x0bexample.com".to_vec()),
    ] {
        let mut bytes = fixture(&address);
        assert_eq!(&request(&key, &endpoint(host)).unwrap()[..], &bytes);
        bytes.extend_from_slice(b"payload");
        let mut input = bytes.as_slice();
        assert_eq!(accept(&mut input, &key).await.unwrap(), endpoint(host));
        assert_eq!(input, b"payload");
    }
    assert_eq!(
        request(&key, &endpoint("EXAMPLE.COM.")).unwrap()[..],
        fixture(b"\x02\x0bexample.com")
    );
}

#[tokio::test]
async fn every_truncated_prefix_and_unsupported_field_is_rejected() {
    let key = uuid(UUID).unwrap();
    let bytes = fixture(b"\x02\x0bexample.com");
    for length in 0..bytes.len() {
        assert!(matches!(
            accept(&mut &bytes[..length], &key).await,
            Err(Error::Io(std::io::ErrorKind::UnexpectedEof))
        ));
    }
    for (index, value) in [
        (0, 1),
        (17, 1),
        (18, 2),
        (18, 3),
        (21, 4),
        (22, 0),
        (23, 0xff),
    ] {
        let mut malformed = bytes.clone();
        malformed[index] = value;
        assert!(matches!(
            accept(&mut malformed.as_slice(), &key).await,
            Err(Error::Protocol)
        ));
    }
    let mut wrong_key = bytes.clone();
    wrong_key[1] ^= 1;
    assert!(matches!(
        accept(&mut wrong_key.as_slice(), &key).await,
        Err(Error::Credential)
    ));
    let mut zero_port = bytes;
    zero_port[19..21].fill(0);
    assert!(matches!(
        accept(&mut zero_port.as_slice(), &key).await,
        Err(Error::Protocol)
    ));
}

#[tokio::test]
async fn request_parser_handles_one_byte_reads() {
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let key = uuid(UUID).unwrap();
    let bytes = fixture(b"\x02\x0bexample.com");
    let (written, accepted) = tokio::join!(writer.write_all(&bytes), accept(&mut reader, &key));
    written.unwrap();
    assert_eq!(accepted.unwrap(), endpoint("example.com"));
}
