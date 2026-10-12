//! Source converters. Every item becomes a member or a [`crate::Skipped`]
//! entry; only an unreadable document, an oversized source or set, or an
//! empty result fails the whole conversion.

mod mihomo;
mod singbox;

use crate::{
    Cidr, Conversion, ConvertError, Entry, Keyword, Name, Reason, RuleSet, report::Collector,
};
use std::{fmt, str::FromStr};

/// Largest accepted source document.
pub const MAX_SOURCE_BYTES: usize = 32 << 20;

/// Input formats; names are the configuration `format` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// sing-box rule-set source JSON, versions 1–3.
    SingBoxSource,
    /// mihomo `behavior: domain` YAML `payload`.
    MihomoDomain,
    /// mihomo `behavior: ipcidr` YAML `payload`.
    MihomoIpcidr,
    /// mihomo `behavior: domain`, `format: text` (meta-rules-dat `.list`).
    MihomoDomainText,
    /// mihomo `behavior: ipcidr`, `format: text`.
    MihomoIpcidrText,
    /// mihomo／Clash `behavior: classical` text, `TYPE,value[,option]`.
    Classical,
    /// An `MWRS` artifact; accepted only if rebuilding it gives the same bytes.
    Meshway,
}

const NAMES: [(Format, &str); 7] = [
    (Format::SingBoxSource, "sing-box-source"),
    (Format::MihomoDomain, "mihomo-domain"),
    (Format::MihomoIpcidr, "mihomo-ipcidr"),
    (Format::MihomoDomainText, "mihomo-domain-text"),
    (Format::MihomoIpcidrText, "mihomo-ipcidr-text"),
    (Format::Classical, "classical"),
    (Format::Meshway, "meshway"),
];

impl Format {
    pub const ALL: [Format; 7] = [
        Self::SingBoxSource,
        Self::MihomoDomain,
        Self::MihomoIpcidr,
        Self::MihomoDomainText,
        Self::MihomoIpcidrText,
        Self::Classical,
        Self::Meshway,
    ];
    pub fn as_str(self) -> &'static str {
        NAMES
            .iter()
            .find(|(f, _)| *f == self)
            .map(|(_, n)| *n)
            .unwrap_or_default()
    }
}
impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Unknown rule-set source format.")]
pub struct UnknownFormat;

impl FromStr for Format {
    type Err = UnknownFormat;
    fn from_str(text: &str) -> Result<Self, UnknownFormat> {
        NAMES
            .iter()
            .find(|(_, n)| *n == text)
            .map(|(f, _)| *f)
            .ok_or(UnknownFormat)
    }
}
impl serde::Serialize for Format {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> serde::Deserialize<'de> for Format {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

pub fn convert(bytes: &[u8], format: Format) -> Result<Conversion, ConvertError> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(ConvertError::TooLarge);
    }
    let mut c = Collector::new();
    match format {
        Format::SingBoxSource => singbox::convert(bytes, &mut c)?,
        Format::MihomoDomain => {
            for (i, item) in mihomo::payload(bytes)?.iter().enumerate() {
                let at = || format!("/payload/{i}");
                match item.as_str() {
                    Some(text) => mihomo::domain(text, at, &mut c),
                    None => c.report.skip(at, Reason::NotString, &mihomo::show(item)),
                }
            }
        }
        Format::MihomoIpcidr => {
            for (i, item) in mihomo::payload(bytes)?.iter().enumerate() {
                let at = || format!("/payload/{i}");
                match item.as_str() {
                    Some(text) => cidr(text, at, &mut c),
                    None => c.report.skip(at, Reason::NotString, &mihomo::show(item)),
                }
            }
        }
        Format::MihomoDomainText => {
            for (line, text) in mihomo::lines(bytes)? {
                mihomo::domain(text, || format!("line {line}"), &mut c);
            }
        }
        Format::MihomoIpcidrText => {
            for (line, text) in mihomo::lines(bytes)? {
                cidr(text, || format!("line {line}"), &mut c);
            }
        }
        Format::Classical => {
            for (line, text) in mihomo::lines(bytes)? {
                mihomo::classical(text, || format!("line {line}"), &mut c);
            }
        }
        Format::Meshway => return rebuild(bytes),
    }
    c.finish()
}

fn rebuild(bytes: &[u8]) -> Result<Conversion, ConvertError> {
    let set = RuleSet::from_vec(bytes.to_vec())
        .map_err(|e| ConvertError::InvalidSource(e.to_string()))?;
    let mut c = Collector::new();
    for d in set.domains() {
        let name = Name::parse(&d.name).map_err(|_| noncanonical())?;
        c.accept(match d.kind {
            crate::DomainKind::Exact => Entry::Exact(name),
            crate::DomainKind::Suffix => Entry::Suffix(name),
            crate::DomainKind::Subdomain => Entry::Subdomain(name),
        });
    }
    for k in set.keywords() {
        c.accept(Entry::Keyword(
            Keyword::parse(k).map_err(|_| noncanonical())?,
        ));
    }
    for r in set.ipv4_ranges() {
        for cidr in Cidr::cover_v4(r) {
            c.accept(Entry::Cidr(cidr));
        }
    }
    for r in set.ipv6_ranges() {
        for cidr in Cidr::cover_v6(r) {
            c.accept(Entry::Cidr(cidr));
        }
    }
    let conversion = c.finish()?;
    if conversion.artifact.bytes() != bytes {
        return Err(noncanonical());
    }
    Ok(conversion)
}

fn noncanonical() -> ConvertError {
    ConvertError::InvalidSource("the artifact is not in canonical form".into())
}

fn cidr(text: &str, at: impl FnOnce() -> String, c: &mut Collector) {
    match Cidr::parse(text) {
        Ok(cidr) => c.accept(Entry::Cidr(cidr)),
        Err(_) => c.report.skip(at, Reason::InvalidCidr, text),
    }
}

fn name(text: &str, kind: fn(Name) -> Entry, at: impl FnOnce() -> String, c: &mut Collector) {
    match Name::parse(text) {
        Ok(n) => c.accept(kind(n)),
        Err(_) => c.report.skip(at, Reason::InvalidName, text),
    }
}

fn keyword(text: &str, at: impl FnOnce() -> String, c: &mut Collector) {
    match Keyword::parse(text) {
        Ok(k) => c.accept(Entry::Keyword(k)),
        Err(_) => c.report.skip(at, Reason::InvalidKeyword, text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_names_round_trip() {
        for f in Format::ALL {
            assert_eq!(f.as_str().parse::<Format>(), Ok(f));
        }
    }
}
