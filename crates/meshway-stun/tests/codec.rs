use std::net::SocketAddr;

use meshway_stun::{
    Error, Fingerprint, MAX_DATAGRAM_SIZE, MessageKind, TransactionId, classify, encode_request,
    encode_response, encode_size_probe, parse_request, parse_response,
};

// Literal protocol test data transcribed from RFC 5769 sections 2.2 and 2.3,
// https://www.rfc-editor.org/rfc/rfc5769#section-2.2 (not produced by our encoder).
const RFC_IPV4: &str = "
    0101003c 2112a442 b7e7a701bc34d686fa87dfae
    8022000b 7465737420766563746f7220
    00200008 0001a147e112a643
    00080014 2b91f599fd9e90c38c7489f92af9ba53f06be7d7
    80280004 c07d4c96";
const RFC_IPV6: &str = "
    01010048 2112a442 b7e7a701bc34d686fa87dfae
    8022000b 7465737420766563746f7220
    00200014 0002a1470113a9faa5d3f179bc25f4b5be d2b9d9
    00080014 a382954e4be67bf11784c97c8292c275bfe3ed41
    80280004 c8fb0b4c";
const TX: TransactionId = TransactionId::from_bytes([
    0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
]);

fn hex(input: &str) -> Vec<u8> {
    let digits: Vec<_> = input.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(digits.len() % 2, 0);
    digits
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].to_digit(16).unwrap() * 16 + pair[1].to_digit(16).unwrap()) as u8)
        .collect()
}

fn endpoint() -> SocketAddr {
    "192.0.2.20:3478".parse().unwrap()
}

fn parse(bytes: &[u8]) -> Result<meshway_stun::MappedAddress, Error> {
    parse_response(TX, endpoint(), endpoint(), bytes)
}

fn set_length(bytes: &mut [u8]) {
    let length = (bytes.len() - 20) as u16;
    bytes[2..4].copy_from_slice(&length.to_be_bytes());
}

fn append(bytes: &mut Vec<u8>, attr: &[u8]) {
    bytes.extend_from_slice(attr);
    set_length(bytes);
}

#[test]
fn rfc5769_golden_responses_are_unauthenticated_observations() {
    for (vector, expected) in [
        (RFC_IPV4, "192.0.2.1:32853"),
        (RFC_IPV6, "[2001:db8:1234:5678:11:2233:4455:6677]:32853"),
    ] {
        let bytes = hex(vector);
        assert_eq!(
            parse(&bytes).unwrap().socket_addr(),
            expected.parse::<SocketAddr>().unwrap()
        );
        assert_eq!(classify(&bytes), Some(MessageKind::SuccessResponse));
        // MI is not checked. Corrupt it and recompute the CRC: this is still
        // only an unauthenticated observation, explicitly not an HMAC claim.
        let mut untrusted_mi = bytes.clone();
        let end = untrusted_mi.len();
        untrusted_mi[end - 9] ^= 0xff;
        let crc = crc32fast::hash(&untrusted_mi[..end - 8]) ^ 0x5354_554e;
        untrusted_mi[end - 4..].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(parse(&untrusted_mi), parse(&bytes));
        let mut no_fingerprint = bytes;
        no_fingerprint.truncate(no_fingerprint.len() - 8);
        set_length(&mut no_fingerprint);
        assert_eq!(parse(&no_fingerprint), Err(Error::Authentication));
    }
}

#[test]
fn exact_source_and_transaction_are_required() {
    let bytes = hex(RFC_IPV4);
    for wrong in [
        "192.0.2.21:3478",
        "192.0.2.20:3479",
        "[::ffff:192.0.2.20]:3478",
    ] {
        assert_eq!(
            parse_response(TX, endpoint(), wrong.parse().unwrap(), &bytes),
            Err(Error::Source)
        );
    }
    assert_eq!(
        parse_response(
            TransactionId::from_bytes([0; 12]),
            endpoint(),
            endpoint(),
            &bytes
        ),
        Err(Error::Transaction)
    );
    let mut wrong_tx = bytes;
    wrong_tx[8] ^= 1;
    assert_eq!(parse(&wrong_tx), Err(Error::Transaction));
}

#[test]
fn strict_header_length_type_and_fingerprint() {
    let bytes = hex(RFC_IPV4);
    for (offset, value) in [(0, 0x81), (1, 0x02), (4, 0), (2, 1), (3, 0x3b)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert!(parse(&bad).is_err(), "offset {offset}");
    }
    for length in 0..bytes.len() {
        assert!(parse(&bytes[..length]).is_err());
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(parse(&bad).is_err());
    let last = bad.len() - 2;
    bad.pop();
    bad[last] ^= 1;
    assert_eq!(parse(&bad), Err(Error::Fingerprint));
    let mut not_last = bytes.clone();
    append(&mut not_last, &[0x80, 0x22, 0, 0]);
    assert_eq!(parse(&not_last), Err(Error::Fingerprint));
    let mut wrong_crc_header = bytes;
    // The CRC covers the original header length including the fingerprint.
    let end = wrong_crc_header.len();
    wrong_crc_header[3] -= 8;
    let crc = crc32fast::hash(&wrong_crc_header[..end - 8]) ^ 0x5354_554e;
    wrong_crc_header[3] += 8;
    wrong_crc_header[end - 4..].copy_from_slice(&crc.to_be_bytes());
    assert_eq!(parse(&wrong_crc_header), Err(Error::Fingerprint));
    assert_eq!(parse(&vec![0; MAX_DATAGRAM_SIZE + 1]), Err(Error::Size));
}

#[test]
fn demux_leaves_mesh_and_unsupported_classes_alone() {
    for prefix in [b"MWADJ".as_slice(), b"MWCIR".as_slice()] {
        let mut bytes = vec![0; 64];
        bytes[..prefix.len()].copy_from_slice(prefix);
        assert_eq!(classify(&bytes), None);
    }
    for ty in [0x0011_u16, 0x0002, 0x0112, 0xc001] {
        let mut bytes = encode_request(TX, Fingerprint::Omit);
        bytes[..2].copy_from_slice(&ty.to_be_bytes());
        assert_eq!(classify(&bytes), None);
        assert_eq!(parse_request(&bytes).unwrap_err(), Error::Header);
    }
}

#[test]
fn encoding_matches_independent_xor_vectors_and_optional_fingerprints() {
    for (ip, attr, length) in [
        ("192.0.2.1:32853", "002000080001a147e112a643", 32),
        (
            "[2001:db8:1234:5678:11:2233:4455:6677]:32853",
            "002000140002a1470113a9faa5d3f179bc25f4b5bed2b9d9",
            44,
        ),
    ] {
        for fingerprint in [Fingerprint::Omit, Fingerprint::Include] {
            let raw = encode_request(TX, fingerprint);
            assert_eq!(&raw[4..8], &hex("2112a442"));
            assert_eq!(&raw[8..20], TX.as_bytes());
            let request = parse_request(&raw).unwrap();
            assert_eq!(request.transaction_id(), TX);
            let response = encode_response(&request, ip.parse().unwrap());
            assert_eq!(&response[20..length], hex(attr));
            assert_eq!(
                response.len(),
                length
                    + if fingerprint == Fingerprint::Include {
                        8
                    } else {
                        0
                    }
            );
            assert_eq!(parse(&response).unwrap().socket_addr(), ip.parse().unwrap());
            assert!(response.len() * 5 <= raw.len() * 11);
        }
    }
}

#[test]
fn unknown_mandatory_attributes_are_bounded_and_optional_padding_is_ignored() {
    let mut bytes = encode_request(TX, Fingerprint::Omit);
    append(
        &mut bytes,
        &hex("80220003 616263ff 00260003 000000ab ffff0000"),
    );
    let request = parse_request(&bytes).unwrap();
    let reply = encode_response(&request, "127.0.0.1:5000".parse().unwrap());
    assert_eq!(reply.len(), 32); // No software/padding echo.
    assert!(parse(&reply).is_ok());
    let mut unknown = encode_request(TX, Fingerprint::Omit);
    append(&mut unknown, &hex("12340000 12340000 45670000"));
    let reply = encode_response(&parse_request(&unknown).unwrap(), endpoint());
    assert_eq!(parse(&reply), Err(Error::ErrorResponse { code: 420 }));
    assert_eq!(&reply[36..], &hex("000a0004 12344567"));
    assert!(reply.len() * 5 <= unknown.len() * 11);
    let mut too_many = encode_request(TX, Fingerprint::Omit);
    for n in 0..17_u16 {
        append(
            &mut too_many,
            &[(0x1000 + n).to_be_bytes(), [0, 0]].concat(),
        );
    }
    assert_eq!(
        parse_request(&too_many).unwrap_err(),
        Error::TooManyUnknownAttributes
    );
    // Exact maximum-size packets are distinct from truncated/oversize input.
    let mut maximum = encode_request(TX, Fingerprint::Omit);
    maximum.extend_from_slice(&[0, 0x26]);
    maximum.extend_from_slice(&((MAX_DATAGRAM_SIZE - 24) as u16).to_be_bytes());
    maximum.resize(MAX_DATAGRAM_SIZE, 0);
    set_length(&mut maximum);
    assert!(parse_request(&maximum).is_ok());
    maximum.extend_from_slice(&[0; 4]);
    set_length(&mut maximum);
    assert_eq!(parse_request(&maximum).unwrap_err(), Error::Size);
}

#[test]
fn malformed_tlv_addresses_and_authoritative_duplicates_are_rejected() {
    let request = parse_request(&encode_request(TX, Fingerprint::Omit)).unwrap();
    let response = encode_response(&request, "192.0.2.1:32853".parse().unwrap());
    for (offset, value) in [(22, 0xff), (23, 9), (24, 1), (25, 3)] {
        let mut bad = response.clone();
        bad[offset] = value;
        assert!(parse(&bad).is_err());
    }
    let mut zero_port = response.clone();
    zero_port[26..28].copy_from_slice(&[0x21, 0x12]);
    assert_eq!(parse(&zero_port), Err(Error::Attribute));
    let mut duplicate = response.clone();
    append(&mut duplicate, &response[20..]);
    assert_eq!(parse(&duplicate), Err(Error::Duplicate));
    for attr in [
        "00090004 00000400",
        "000a0002 12340000",
        "00010008 00018055c0000201",
    ] {
        let mut duplicate = encode_request(TX, Fingerprint::Omit);
        append(&mut duplicate, &hex(attr));
        append(&mut duplicate, &hex(attr));
        assert_eq!(parse_request(&duplicate).unwrap_err(), Error::Duplicate);
    }
    let mut duplicate_fp = encode_request(TX, Fingerprint::Include);
    append(&mut duplicate_fp, &hex("80280004 00000000"));
    assert!(parse_request(&duplicate_fp).is_err());
    for attr in [
        "80220001",
        "8022ffff",
        "00200004 0001a147",
        "00090004 00010400",
        "00090005 00000400ff000000",
    ] {
        let mut bad = encode_request(TX, Fingerprint::Omit);
        append(&mut bad, &hex(attr));
        assert!(parse_request(&bad).is_err(), "{attr}");
    }
    // A structurally valid response-only attribute in a request yields a
    // bounded generic 400, never an echo of the claimed mapped address.
    let mut bad_request = encode_request(TX, Fingerprint::Omit);
    append(&mut bad_request, &response[20..]);
    let error = encode_response(&parse_request(&bad_request).unwrap(), endpoint());
    assert_eq!(parse(&error), Err(Error::ErrorResponse { code: 400 }));
    assert_eq!(error.len(), 40);
}

#[test]
fn authenticated_requests_and_integrity_boundary_abuse_are_not_accepted() {
    for attr in [
        "00060004 75736572",
        "00140000",
        "00150000",
        "80020000",
        "00080014 0000000000000000000000000000000000000000",
    ] {
        let mut bytes = encode_request(TX, Fingerprint::Omit);
        append(&mut bytes, &hex(attr));
        assert_eq!(parse_request(&bytes).unwrap_err(), Error::Authentication);
    }
    let request = parse_request(&encode_request(TX, Fingerprint::Omit)).unwrap();
    let response = encode_response(&request, endpoint());
    let mut bytes = response[..20].to_vec();
    append(
        &mut bytes,
        &hex("00080014 0000000000000000000000000000000000000000"),
    );
    append(&mut bytes, &response[20..]);
    assert_eq!(parse(&bytes), Err(Error::Authentication));
    let mut unknown_response = response;
    append(&mut unknown_response, &hex("12340000"));
    assert_eq!(parse(&unknown_response), Err(Error::UnknownAttribute));
}

#[test]
fn sha256_integrity_has_bounded_length_and_follows_sha1_if_both_present() {
    let request = parse_request(&encode_request(TX, Fingerprint::Omit)).unwrap();
    let response = encode_response(&request, endpoint());
    let sha1 = hex("00080014 0000000000000000000000000000000000000000");
    let sha256 = hex("001c0020 0000000000000000000000000000000000000000000000000000000000000000");
    for attrs in [sha256.clone(), [sha1.clone(), sha256.clone()].concat()] {
        let mut bytes = response.clone();
        append(&mut bytes, &attrs);
        let offset = bytes.len();
        append(&mut bytes, &hex("80280004 00000000"));
        let crc = crc32fast::hash(&bytes[..offset]) ^ 0x5354_554e;
        bytes[offset + 4..].copy_from_slice(&crc.to_be_bytes());
        assert!(parse(&bytes).is_ok());
    }
    for attrs in [
        [sha256.clone(), sha1.clone()].concat(),
        [sha1.clone(), sha1].concat(),
        [sha256.clone(), sha256].concat(),
        hex("001c0004 00000000"),
    ] {
        let mut bytes = response.clone();
        append(&mut bytes, &attrs);
        assert!(parse(&bytes).is_err());
    }
}

#[test]
fn a_size_probe_is_an_ordinary_request_of_exactly_its_size() {
    for size in [36, 1252, 1328, 1472] {
        let probe = encode_size_probe(TX, size).unwrap();
        assert_eq!(probe.len(), size);
        assert_eq!(classify(&probe), Some(MessageKind::Request));
        // The filler is comprehension-optional: the request is answered normally.
        let request = parse_request(&probe).unwrap();
        let response = encode_response(&request, endpoint());
        assert!(response.len() < 64);
        assert_eq!(
            parse_response(TX, endpoint(), endpoint(), &response)
                .unwrap()
                .socket_addr(),
            endpoint()
        );
    }
    for size in [0, 35, 1473, MAX_DATAGRAM_SIZE + 4] {
        assert!(encode_size_probe(TX, size).is_none(), "{size}");
    }
}
