//! Inline lists from configuration (`ruleSets.<key>` of type `inline` and the
//! routing rule `match` lists) go through the same entry parsers and builder
//! as remote sources.

use crate::{
    Cidr, Conversion, ConvertError, Entry, LoadError, Name, Reason, RuleSet, Skipped,
    report::Collector,
};

/// Raw configuration lists; field names follow the configuration document.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inline<'a> {
    /// `domain`: exact names.
    pub domain: &'a [String],
    /// `domainSuffix`: the name and its subdomains.
    pub domain_suffix: &'a [String],
    /// `domainSubdomain`: subdomains only.
    pub domain_subdomain: &'a [String],
    /// `ipCIDR`.
    pub ip_cidr: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InlineError {
    /// Configuration lists must convert completely; this is the first item
    /// that did not.
    #[error("The inline entry at {} is invalid ({}).", .0.at, .0.reason)]
    Entry(Skipped),
    #[error(transparent)]
    Convert(#[from] ConvertError),
    #[error(transparent)]
    Load(#[from] LoadError),
}

type NameField<'a> = (&'static str, &'a [String], fn(Name) -> Entry);

/// Converts every list and reports each invalid item at `/<field>/<index>`.
pub fn convert(inline: &Inline<'_>) -> Result<Conversion, ConvertError> {
    let mut c = Collector::new();
    let names: [NameField<'_>; 3] = [
        ("domain", inline.domain, Entry::Exact),
        ("domainSuffix", inline.domain_suffix, Entry::Suffix),
        ("domainSubdomain", inline.domain_subdomain, Entry::Subdomain),
    ];
    for (field, list, kind) in names {
        for (i, raw) in list.iter().enumerate() {
            match Name::parse(raw) {
                Ok(n) => c.accept(kind(n)),
                Err(_) => c
                    .report
                    .skip(|| format!("/{field}/{i}"), Reason::InvalidName, raw),
            }
        }
    }
    for (i, raw) in inline.ip_cidr.iter().enumerate() {
        match Cidr::parse(raw) {
            Ok(cidr) => c.accept(Entry::Cidr(cidr)),
            Err(_) => c
                .report
                .skip(|| format!("/ipCIDR/{i}"), Reason::InvalidCidr, raw),
        }
    }
    c.finish()
}

impl RuleSet {
    /// In-memory handle for configuration lists, which must convert without
    /// any skipped item and must not all be empty.
    pub fn from_inline(inline: &Inline<'_>) -> Result<Self, InlineError> {
        let conversion = convert(inline)?;
        if let Some(first) = conversion.report.items.into_iter().next() {
            return Err(InlineError::Entry(first));
        }
        Ok(conversion.artifact.open()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DomainHit;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn inline_lists_share_the_matcher() {
        let (d, s, sub, ip) = (
            strings(&["Login.Partner.example."]),
            strings(&["corp.example"]),
            strings(&["cdn.example"]),
            strings(&["10.20.0.0/16"]),
        );
        let set = RuleSet::from_inline(&Inline {
            domain: &d,
            domain_suffix: &s,
            domain_subdomain: &sub,
            ip_cidr: &ip,
        })
        .unwrap();
        assert_eq!(
            set.match_domain("login.partner.example"),
            Some(DomainHit::Exact)
        );
        assert_eq!(
            set.match_domain("a.corp.example"),
            Some(DomainHit::Suffix { len: 12 })
        );
        assert_eq!(
            set.match_domain("corp.example"),
            Some(DomainHit::Suffix { len: 12 })
        );
        assert_eq!(
            set.match_domain("x.cdn.example"),
            Some(DomainHit::Subdomain { len: 11 })
        );
        assert_eq!(set.match_domain("cdn.example"), None);
        assert!(set.match_ip("10.20.3.4".parse().unwrap()));
        assert!(!set.match_ip("10.21.0.0".parse().unwrap()));
    }

    #[test]
    fn domains_page_by_reversed_name_suffix() {
        // Exact names: a suffix member would make the names below it redundant.
        let suffix = strings(&["example.org"]);
        let exact = strings(&["openai.com", "ai.com", "api.openai.com", "chat.openai.com"]);
        let set = RuleSet::from_inline(&Inline {
            domain: &exact,
            domain_suffix: &suffix,
            ..Inline::default()
        })
        .unwrap();
        let names = |suffix: &str, after: Option<&str>| -> Vec<String> {
            set.domains_ending_with(suffix, after)
                .map(|entry| entry.name)
                .collect()
        };
        assert_eq!(
            names("openai.com", None),
            ["openai.com", "api.openai.com", "chat.openai.com"]
        );
        assert_eq!(
            names(".openai.com", None),
            ["api.openai.com", "chat.openai.com"]
        );
        assert_eq!(names("ai.com", None).len(), 4);
        assert_eq!(
            names("openai.com", Some("api.openai.com")),
            ["chat.openai.com"]
        );
        // The order is by reversed name: every `.org` name sorts before `.com`.
        assert_eq!(names("com", Some("zzz.org")).len(), 4);
        assert_eq!(names("org", Some("x.com")), Vec::<String>::new());
        assert_eq!(set.domains().count(), 5);
    }

    #[test]
    fn inline_rejects_any_invalid_item_and_empty_lists() {
        let (good, bad) = (strings(&["ok.example"]), strings(&["10.0.0.0"]));
        let err = RuleSet::from_inline(&Inline {
            domain: &good,
            ip_cidr: &bad,
            ..Inline::default()
        })
        .unwrap_err();
        let InlineError::Entry(item) = err else {
            panic!("{err:?}")
        };
        assert_eq!(
            (item.at.as_str(), item.reason),
            ("/ipCIDR/0", Reason::InvalidCidr)
        );
        assert_eq!(
            RuleSet::from_inline(&Inline::default()).unwrap_err(),
            InlineError::Convert(ConvertError::Empty)
        );
    }
}
