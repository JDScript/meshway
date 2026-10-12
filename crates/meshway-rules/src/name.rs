use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

/// Canonical DNS name: UTS 46 ToASCII (`idna::domain_to_ascii`), lowercase,
/// trailing dots removed, labels 1–63 bytes of `[a-z0-9-]` not starting or
/// ending with `-`, at most 253 bytes. This is the Server converter's
/// `canonical_domain` rule unchanged. It is stricter than mihomo and sing-box:
/// names with `_` and punycode labels that do not decode to a valid IDNA label
/// (for example `xn--wcvs22d1m.hk` in meta-rules-dat) are rejected and
/// reported, where upstream would match them literally.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The DNS name is not a valid canonical name.")]
pub struct InvalidName;

impl Name {
    pub fn parse(raw: &str) -> Result<Self, InvalidName> {
        let name = idna::domain_to_ascii(raw.trim_end_matches('.'))
            .map_err(|_| InvalidName)?
            .to_ascii_lowercase();
        let valid = !name.is_empty()
            && name.len() <= 253
            && name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        if valid {
            Ok(Self(name))
        } else {
            Err(InvalidName)
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
}
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// Substring literal matched against query names: 1–253 bytes of printable
/// ASCII without spaces, lowercased. It is wider than a canonical name on
/// purpose: query names may carry `_` (`_dmarc.example.com`) and upstream
/// keyword lists contain such literals.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Keyword(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The keyword is empty, too long or contains bytes no canonical name has.")]
pub struct InvalidKeyword;

impl Keyword {
    pub fn parse(raw: &str) -> Result<Self, InvalidKeyword> {
        let keyword = raw.to_ascii_lowercase();
        if keyword.is_empty() || keyword.len() > 253 || !keyword.bytes().all(keyword_byte) {
            return Err(InvalidKeyword);
        }
        Ok(Self(keyword))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

pub(crate) fn name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.'
}

pub(crate) fn keyword_byte(b: u8) -> bool {
    b.is_ascii_graphic() && !b.is_ascii_uppercase()
}

/// `address/bits` with host bits cleared (`10.4.3.2/8` is `10.0.0.0/8`), the
/// Server converter's prefix rule unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cidr {
    V4(Ipv4Addr, u8),
    V6(Ipv6Addr, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The value is not an IPv4 or IPv6 `address/bits` prefix.")]
pub struct InvalidCidr;

impl Cidr {
    pub fn parse(raw: &str) -> Result<Self, InvalidCidr> {
        let (address, bits) = raw.split_once('/').ok_or(InvalidCidr)?;
        let address: IpAddr = address.parse().map_err(|_| InvalidCidr)?;
        let bits: u8 = bits.parse().map_err(|_| InvalidCidr)?;
        match address {
            IpAddr::V4(ip) if bits <= 32 => {
                let mask = u32::MAX.checked_shl(u32::from(32 - bits)).unwrap_or(0);
                Ok(Self::V4(Ipv4Addr::from(u32::from(ip) & mask), bits))
            }
            IpAddr::V6(ip) if bits <= 128 => {
                let mask = u128::MAX.checked_shl(u32::from(128 - bits)).unwrap_or(0);
                Ok(Self::V6(Ipv6Addr::from(u128::from(ip) & mask), bits))
            }
            _ => Err(InvalidCidr),
        }
    }
    pub(crate) fn v4_range(self) -> Option<(u32, u32)> {
        match self {
            Self::V4(ip, bits) => {
                let host = u32::MAX.checked_shr(u32::from(bits)).unwrap_or(0);
                Some((u32::from(ip), u32::from(ip) | host))
            }
            Self::V6(..) => None,
        }
    }
    pub(crate) fn v6_range(self) -> Option<(u128, u128)> {
        match self {
            Self::V6(ip, bits) => {
                let host = u128::MAX.checked_shr(u32::from(bits)).unwrap_or(0);
                Some((u128::from(ip), u128::from(ip) | host))
            }
            Self::V4(..) => None,
        }
    }
}
impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(ip, bits) => write!(f, "{ip}/{bits}"),
            Self::V6(ip, bits) => write!(f, "{ip}/{bits}"),
        }
    }
}
impl fmt::Debug for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// One normalized rule-set member; see the matching table in the design.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Entry {
    /// The name itself only.
    Exact(Name),
    /// The name and every name below it at a label boundary.
    Suffix(Name),
    /// Only names strictly below it (`.example.com` in sing-box／mihomo).
    Subdomain(Name),
    /// Any canonical name containing the literal.
    Keyword(Keyword),
    /// Any destination address inside the prefix.
    Cidr(Cidr),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_server_rule() {
        let ok = |raw: &str| Name::parse(raw).map(Name::into_string);
        assert_eq!(ok("Github.com."), Ok("github.com".into()));
        assert_eq!(ok("a.example.com.."), Ok("a.example.com".into()));
        assert_eq!(ok("中国.cn"), Ok("xn--fiqs8s.cn".into()));
        assert_eq!(ok("XN--FIQS8S.cn"), Ok("xn--fiqs8s.cn".into()));
        for bad in [
            "",
            ".",
            "a..b",
            "-a.com",
            "a-.com",
            "_dmarc.example.com",
            "*.example.com",
            "xn--wcvs22d1m.hk",
            &format!("{}.com", "a".repeat(64)),
            &format!("{}.com", vec!["a".repeat(60); 5].join(".")),
        ] {
            assert_eq!(Name::parse(bad), Err(InvalidName), "{bad}");
        }
    }

    #[test]
    fn keywords_are_lowercase_printable_ascii() {
        assert_eq!(Keyword::parse("GooGle").unwrap().as_str(), "google");
        assert_eq!(Keyword::parse("_Vmind").unwrap().as_str(), "_vmind");
        for bad in ["", "a b", "中国", "a\tb", &"a".repeat(254)] {
            assert_eq!(Keyword::parse(bad), Err(InvalidKeyword), "{bad}");
        }
    }

    #[test]
    fn prefixes_clear_host_bits() {
        assert_eq!(Cidr::parse("10.4.3.2/8").unwrap().to_string(), "10.0.0.0/8");
        assert_eq!(Cidr::parse("::1/0").unwrap().to_string(), "::/0");
        assert_eq!(
            Cidr::parse("0.0.0.0/0").unwrap().v4_range(),
            Some((0, u32::MAX))
        );
        assert_eq!(
            Cidr::parse("2001:db8::7/128").unwrap().v6_range(),
            Some((0x2001_0db8_u128 << 96 | 7, 0x2001_0db8_u128 << 96 | 7))
        );
        for bad in [
            "10.0.0.0",
            "10.0.0.0/33",
            "::/129",
            "10.0.0.0/x",
            "fe80::1%eth0/64",
        ] {
            assert_eq!(Cidr::parse(bad), Err(InvalidCidr), "{bad}");
        }
    }
}
