use super::*;
use tokio::io::AsyncWriteExt;

const HASH: &[u8] = b"d63dc919e201d7bc4c825630d2cf25fdc93d4b2f0d46706d29038d01";

fn endpoint(host: &str) -> Endpoint {
    Endpoint {
        host: host.into(),
        port: Port::new(443).unwrap(),
    }
}

fn fixture(address: &[u8]) -> Vec<u8> {
    let mut bytes = HASH.to_vec();
    bytes.extend_from_slice(b"\r\n\x01");
    bytes.extend_from_slice(address);
    bytes.extend_from_slice(b"\x01\xbb\r\n");
    bytes
}

#[tokio::test]
async fn reference_sha224_lowercase_and_socks_address_port_order() {
    let key = key("password").unwrap();
    assert_eq!(key.as_slice(), HASH);
    for (host, address) in [
        ("192.0.2.1", vec![1, 192, 0, 2, 1]),
        (
            "2001:db8::1",
            vec![4, 0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        ),
        ("example.com", b"\x03\x0bexample.com".to_vec()),
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
        fixture(b"\x03\x0bexample.com")
    );
}

#[tokio::test]
async fn rejects_all_truncations_credentials_commands_address_types_and_crlf() {
    let key = key("password").unwrap();
    let bytes = fixture(b"\x03\x0bexample.com");
    for length in 0..bytes.len() {
        assert!(matches!(
            accept(&mut &bytes[..length], &key).await,
            Err(Error::Io(std::io::ErrorKind::UnexpectedEof))
        ));
    }
    for (index, value) in [
        (56, b'x'),
        (57, b'x'),
        (58, 3),
        (58, 0x7f),
        (59, 2),
        (60, 0),
        (61, 0xff),
        (bytes.len() - 1, b'x'),
    ] {
        let mut malformed = bytes.clone();
        malformed[index] = value;
        assert!(matches!(
            accept(&mut malformed.as_slice(), &key).await,
            Err(Error::Protocol)
        ));
    }
    let mut uppercase = bytes.clone();
    uppercase[0] = b'D';
    assert!(matches!(
        accept(&mut uppercase.as_slice(), &key).await,
        Err(Error::Credential)
    ));
    let mut zero_port = bytes;
    let len = zero_port.len();
    zero_port[len - 4..len - 2].fill(0);
    assert!(matches!(
        accept(&mut zero_port.as_slice(), &key).await,
        Err(Error::Protocol)
    ));
}

#[tokio::test]
async fn parser_handles_fragmented_request() {
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let key = key("password").unwrap();
    let bytes = fixture(b"\x03\x0bexample.com");
    let (written, accepted) = tokio::join!(writer.write_all(&bytes), accept(&mut reader, &key));
    written.unwrap();
    assert_eq!(accepted.unwrap(), endpoint("example.com"));
}
