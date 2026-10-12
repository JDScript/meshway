use crate::{Artifact, BuildError, Entry};
use std::{collections::BTreeMap, fmt, str::FromStr};

/// Skipped source items kept verbatim; the rest are only counted.
pub const MAX_REPORT_ITEMS: usize = 1000;
const MAX_ITEM_CHARS: usize = 256;
const MAX_FIELD_CHARS: usize = 64;

/// Why one source item did not become a member. Serialized as the design's
/// reason strings (`regex`, `unsupportedField:<name>`, ...).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// `domain_regex`／`DOMAIN-REGEX`: never lowered in v1.
    Regex,
    /// Not a canonical DNS name (see [`crate::Name`]).
    InvalidName,
    InvalidKeyword,
    InvalidCidr,
    /// mihomo `*` or a `+` other than the leading `+.`.
    Wildcard,
    /// A sing-box rule ANDing name and IP fields.
    MixedRule,
    /// A sing-box rule without conditions (it matches everything upstream).
    EmptyRule,
    /// A list item that is not a string.
    NotString,
    /// A sing-box rule field outside the supported set; the whole rule is skipped.
    UnsupportedField(String),
    /// A classical rule type (or option) with no equivalent.
    Classical(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regex => f.write_str("regex"),
            Self::InvalidName => f.write_str("invalidName"),
            Self::InvalidKeyword => f.write_str("invalidKeyword"),
            Self::InvalidCidr => f.write_str("invalidCidr"),
            Self::Wildcard => f.write_str("wildcard"),
            Self::MixedRule => f.write_str("mixedRule"),
            Self::EmptyRule => f.write_str("emptyRule"),
            Self::NotString => f.write_str("notString"),
            Self::UnsupportedField(name) => write!(f, "unsupportedField:{name}"),
            Self::Classical(kind) => write!(f, "classical:{kind}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Unknown skip reason.")]
pub struct UnknownReason;

impl FromStr for Reason {
    type Err = UnknownReason;
    fn from_str(text: &str) -> Result<Self, UnknownReason> {
        Ok(match text {
            "regex" => Self::Regex,
            "invalidName" => Self::InvalidName,
            "invalidKeyword" => Self::InvalidKeyword,
            "invalidCidr" => Self::InvalidCidr,
            "wildcard" => Self::Wildcard,
            "mixedRule" => Self::MixedRule,
            "emptyRule" => Self::EmptyRule,
            "notString" => Self::NotString,
            _ => match text.split_once(':') {
                Some(("unsupportedField", name)) => Self::UnsupportedField(name.to_owned()),
                Some(("classical", kind)) => Self::Classical(kind.to_owned()),
                _ => return Err(UnknownReason),
            },
        })
    }
}
impl serde::Serialize for Reason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> serde::Deserialize<'de> for Reason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// One skipped item: where it was (`/rules/3/domain_regex/0`, `/payload/7`,
/// `line 12`, `/domainSuffix/2`), why, and its text (truncated).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Skipped {
    pub at: String,
    pub reason: Reason,
    pub entry: String,
}

/// Accepted source items by the member kind they became (before
/// minimization; duplicates included).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Converted {
    pub exact: u64,
    pub suffix: u64,
    pub subdomain: u64,
    pub keyword: u64,
    pub cidr: u64,
}

impl Converted {
    pub fn total(&self) -> u64 {
        self.exact + self.suffix + self.subdomain + self.keyword + self.cidr
    }
}

/// Counts of a conversion; what a catalog entry or resource view shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Summary {
    pub converted: Converted,
    /// Accepted name／keyword items dropped as duplicates or covered by
    /// another member (`Artifact::redundant`).
    pub redundant: u64,
    pub unconverted: BTreeMap<Reason, u64>,
}

impl Summary {
    pub fn skipped(&self) -> u64 {
        self.unconverted.values().sum()
    }
}

/// Every source item is either converted or skipped with a reason; the first
/// [`MAX_REPORT_ITEMS`] skipped items are kept verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    pub summary: Summary,
    pub items: Vec<Skipped>,
}

impl Report {
    pub(crate) fn accept(&mut self, entry: &Entry) {
        let c = &mut self.summary.converted;
        match entry {
            Entry::Exact(_) => c.exact += 1,
            Entry::Suffix(_) => c.suffix += 1,
            Entry::Subdomain(_) => c.subdomain += 1,
            Entry::Keyword(_) => c.keyword += 1,
            Entry::Cidr(_) => c.cidr += 1,
        }
    }
    pub(crate) fn skip(&mut self, at: impl FnOnce() -> String, reason: Reason, entry: &str) {
        let reason = match reason {
            Reason::UnsupportedField(name) => {
                Reason::UnsupportedField(truncate(&name, MAX_FIELD_CHARS))
            }
            Reason::Classical(kind) => Reason::Classical(truncate(&kind, MAX_FIELD_CHARS)),
            other => other,
        };
        *self.summary.unconverted.entry(reason.clone()).or_default() += 1;
        if self.items.len() < MAX_REPORT_ITEMS {
            self.items.push(Skipped {
                at: at(),
                reason,
                entry: truncate(entry, MAX_ITEM_CHARS),
            });
        }
    }
}

fn truncate(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// A converted artifact with its report.
#[derive(Clone, Debug)]
pub struct Conversion {
    pub artifact: Artifact,
    pub report: Report,
}

/// Whole-conversion failures; single items never fail a conversion.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    /// The source exceeds the download limit.
    #[error("The source exceeds the size limit.")]
    TooLarge,
    /// The document itself cannot be parsed as the declared format.
    #[error("The source is not a valid document of this format: {0}")]
    InvalidSource(String),
    /// The converted set exceeds an artifact limit.
    #[error(transparent)]
    Budget(#[from] BuildError),
    /// Nothing was convertible.
    #[error("No source item could be converted.")]
    Empty,
}

/// Inserts accepted entries into a builder and finishes the conversion.
pub(crate) struct Collector {
    pub(crate) builder: crate::Builder,
    pub(crate) report: Report,
}

impl Collector {
    pub(crate) fn new() -> Self {
        Self {
            builder: crate::Builder::new(),
            report: Report::default(),
        }
    }
    pub(crate) fn accept(&mut self, entry: Entry) {
        self.report.accept(&entry);
        self.builder.insert(entry);
    }
    pub(crate) fn finish(self) -> Result<Conversion, ConvertError> {
        let Self {
            builder,
            mut report,
        } = self;
        if report.summary.converted.total() == 0 {
            return Err(ConvertError::Empty);
        }
        let artifact = builder.build()?;
        report.summary.redundant = artifact.redundant();
        Ok(Conversion { artifact, report })
    }
}
