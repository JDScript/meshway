//! Converter semantics, including every case of the current Server converter
//! (`apps/meshway-server/src/resources/rulesets.rs`). Where the R1 decision
//! changes an outcome (per-item reports instead of whole failure, keyword and
//! subdomain support) the test states the old result next to the new one.
#![cfg(feature = "convert")]

use meshway_rules::{
    ConvertError, Counts, DomainHit, Reason, RuleSet,
    convert::{Format, convert},
};
use std::net::IpAddr;

fn set(source: &[u8], format: Format) -> (RuleSet, meshway_rules::Report) {
    let c = convert(source, format).unwrap();
    (c.artifact.open().unwrap(), c.report)
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn reasons(report: &meshway_rules::Report) -> Vec<(String, Reason)> {
    report
        .items
        .iter()
        .map(|i| (i.at.clone(), i.reason.clone()))
        .collect()
}

#[test]
fn server_case_keyword_rule_now_converts() {
    // Old: unknown field `domain_keyword` failed the conversion. New: keyword
    // members match exactly what sing-box matches.
    let (s, report) = set(
        br#"{"version":1,"rules":[{"domain_keyword":["Example"]}]}"#,
        Format::SingBoxSource,
    );
    assert_eq!(report.summary.converted.keyword, 1);
    assert_eq!(
        s.match_domain("myexample.org"),
        Some(DomainHit::Keyword { index: 0 })
    );
    assert_eq!(s.keywords().collect::<Vec<_>>(), ["example"]);
    assert_eq!(s.match_domain("exampl.org"), None);
}

#[test]
fn server_case_mixed_rule_is_reported_not_widened() {
    // Old: whole failure. New: both items reported `mixedRule`; nothing else
    // converts, so the conversion is empty.
    let source =
        br#"{"version":1,"rules":[{"domain_suffix":["example.com"],"ip_cidr":["10.0.0.0/8"]}]}"#;
    assert_eq!(
        convert(source, Format::SingBoxSource).unwrap_err(),
        ConvertError::Empty
    );
    let source = br#"{"version":2,"rules":[{"domain":["a.example.com"],"ip_cidr":["10.0.0.0/8"]},{"domain":"ok.example"}]}"#;
    let (s, report) = set(source, Format::SingBoxSource);
    assert_eq!(
        reasons(&report),
        [
            ("/rules/0/domain/0".to_owned(), Reason::MixedRule),
            ("/rules/0/ip_cidr/0".to_owned(), Reason::MixedRule),
        ]
    );
    assert_eq!(s.match_domain("a.example.com"), None);
    assert!(!s.match_ip(ip("10.1.1.1")));
    assert_eq!(s.match_domain("ok.example"), Some(DomainHit::Exact));
}

#[test]
fn server_case_mihomo_wildcards() {
    // `*.` has no equivalent (old: whole failure; new: reported, empty).
    assert_eq!(
        convert(b"payload:\n  - '*.example.com'", Format::MihomoDomain).unwrap_err(),
        ConvertError::Empty
    );
    let (s, report) = set(
        b"payload:\n  - '*.example.com'\n  - 'a+b.example'\n  - '+.ok.example'",
        Format::MihomoDomain,
    );
    assert_eq!(
        report.summary.unconverted.get(&Reason::Wildcard).copied(),
        Some(2)
    );
    assert_eq!(
        s.match_domain("x.ok.example"),
        Some(DomainHit::Suffix { len: 10 })
    );
    // Old: `.example.com` failed. New (user decision): subdomain member.
    let (s, _) = set(b"payload:\n  - '.example.com'", Format::MihomoDomain);
    assert_eq!(s.match_domain("example.com"), None);
    assert_eq!(
        s.match_domain("a.b.example.com"),
        Some(DomainHit::Subdomain { len: 11 })
    );
    // `+.Example.com` is a suffix only.
    let (s, _) = set(b"payload:\n  - '+.Example.com'", Format::MihomoDomain);
    assert_eq!(
        s.counts(),
        Counts {
            suffix: 1,
            ..Counts::default()
        }
    );
    assert_eq!(s.domains().next().unwrap().name, "example.com");
}

#[test]
fn server_case_exact_names_stay_exact() {
    let (s, report) = set(
        b"payload:\n  - 'google.com'\n  - '+.google.com'\n  - 'Github.com.'\n  - 'github.com'",
        Format::MihomoDomain,
    );
    // Old output kept exact `google.com` beside suffix `google.com`; the
    // suffix already matches it, so the canonical set drops it.
    assert_eq!((s.counts().exact, s.counts().suffix), (1, 1));
    assert_eq!(report.summary.redundant, 2);
    assert_eq!(s.match_domain("github.com"), Some(DomainHit::Exact));
    assert_eq!(s.match_domain("a.github.com"), None);
    assert_eq!(
        s.match_domain("google.com"),
        Some(DomainHit::Suffix { len: 10 })
    );

    let (s, _) = set(
        br#"{"version":2,"rules":[{"domain":["a.example.com"],"domain_suffix":["b.example.com"]},{"ip_cidr":["10.0.0.0/8"]}]}"#,
        Format::SingBoxSource,
    );
    assert_eq!(s.match_domain("a.example.com"), Some(DomainHit::Exact));
    assert_eq!(s.match_domain("x.a.example.com"), None);
    assert_eq!(
        s.match_domain("x.b.example.com"),
        Some(DomainHit::Suffix { len: 13 })
    );
    assert!(s.match_ip(ip("10.255.0.1")));
}

#[test]
fn server_case_prefixes_are_canonical_and_deduplicated() {
    let (s, _) = set(
        b"payload:\n  - 10.4.3.2/8\n  - 10.0.0.0/8",
        Format::MihomoIpcidr,
    );
    let ranges: Vec<_> = s.ipv4_ranges().map(|r| (*r.start(), *r.end())).collect();
    assert_eq!(ranges, [(ip_v4("10.0.0.0"), ip_v4("10.255.255.255"))]);
}

fn ip_v4(text: &str) -> std::net::Ipv4Addr {
    text.parse().unwrap()
}

#[test]
fn sing_box_items_are_reported_one_by_one() {
    let source = br#"{"version":3,"rules":[
        {"domain":"single.example","domain_suffix":[".sub.example","suffix.example","bad_name.example"]},
        {"domain_regex":["^a\\.example$"]},
        {"ip_cidr":["192.0.2.0/24","2001:db8::/32","192.0.2.1",7]},
        {"invert":true,"domain":["inverted.example"]},
        {"type":"logical","mode":"and","rules":[]},
        {"type":"default"},
        {"domain_keyword":["a b"]}
    ]}"#;
    let (s, report) = set(source, Format::SingBoxSource);
    assert_eq!(
        reasons(&report),
        [
            ("/rules/0/domain_suffix/2".to_owned(), Reason::InvalidName),
            ("/rules/1/domain_regex/0".to_owned(), Reason::Regex),
            ("/rules/2/ip_cidr/2".to_owned(), Reason::InvalidCidr),
            ("/rules/2/ip_cidr/3".to_owned(), Reason::NotString),
            (
                "/rules/3/domain/0".to_owned(),
                Reason::UnsupportedField("invert".into())
            ),
            (
                "/rules/3/invert/0".to_owned(),
                Reason::UnsupportedField("invert".into())
            ),
            (
                "/rules/4/mode/0".to_owned(),
                Reason::UnsupportedField("mode".into())
            ),
            (
                "/rules/4/type/0".to_owned(),
                Reason::UnsupportedField("mode".into())
            ),
            ("/rules/5".to_owned(), Reason::EmptyRule),
            (
                "/rules/6/domain_keyword/0".to_owned(),
                Reason::InvalidKeyword
            ),
        ]
    );
    assert_eq!(report.summary.converted.total(), 5);
    assert_eq!(s.match_domain("single.example"), Some(DomainHit::Exact));
    assert_eq!(
        s.match_domain("a.sub.example"),
        Some(DomainHit::Subdomain { len: 11 })
    );
    assert_eq!(s.match_domain("sub.example"), None);
    assert_eq!(s.match_domain("inverted.example"), None);
    assert!(s.match_ip(ip("2001:db8:ffff::1")));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["summary"]["unconverted"]["unsupportedField:invert"], 2);
}

#[test]
fn whole_document_failures() {
    let bad = |source: &[u8], format| {
        matches!(convert(source, format), Err(ConvertError::InvalidSource(_)))
    };
    assert!(bad(
        br#"{"version":4,"rules":[{"domain":"a.example"}]}"#,
        Format::SingBoxSource
    ));
    assert!(bad(
        br#"{"version":1,"rules":[],"extra":1}"#,
        Format::SingBoxSource
    ));
    assert!(bad(
        br#"{"version":1,"rules":["a.example"]}"#,
        Format::SingBoxSource
    ));
    assert!(bad(b"payload: 3", Format::MihomoDomain));
    assert!(bad(b"payload: []\nother: 1", Format::MihomoIpcidr));
    assert!(bad(b"\xff\xfe", Format::MihomoDomainText));
    assert!(bad(b"MWRULES\0", Format::Meshway));
    assert_eq!(
        convert(&vec![b' '; (32 << 20) + 1], Format::MihomoDomainText).unwrap_err(),
        ConvertError::TooLarge
    );
}

#[test]
fn text_and_yaml_payloads_are_the_same_set() {
    let yaml = convert(
        b"payload:\n  - '+.openai.com'\n  - chatgpt.com\n  - '.oaiusercontent.com'",
        Format::MihomoDomain,
    )
    .unwrap();
    let text = convert(
        b"# comment\n\n  chatgpt.com  \n.oaiusercontent.com\n+.openai.com\n",
        Format::MihomoDomainText,
    )
    .unwrap();
    assert_eq!(yaml.artifact.digest(), text.artifact.digest());
    let sing = convert(
        br#"{"version":1,"rules":[{"domain_suffix":["openai.com",".oaiusercontent.com"]},{"domain":"chatgpt.com"}]}"#,
        Format::SingBoxSource,
    )
    .unwrap();
    assert_eq!(sing.artifact.bytes(), yaml.artifact.bytes());
    let cidrs = convert(b"10.0.0.0/9\n10.128.0.0/9\n", Format::MihomoIpcidrText).unwrap();
    let merged = convert(b"payload: ['10.0.0.0/8']", Format::MihomoIpcidr).unwrap();
    assert_eq!(cidrs.artifact.digest(), merged.artifact.digest());
}

#[test]
fn classical_lines() {
    let (s, report) = set(
        b"DOMAIN,exact.example\nDOMAIN-SUFFIX,suffix.example\nDOMAIN-KEYWORD,track\n\
          DOMAIN-REGEX,^ads\nIP-CIDR,192.0.2.0/24,no-resolve\nIP-CIDR6,2001:db8::/32\n\
          IP-ASN,13335,no-resolve\nPROCESS-NAME,curl\nDOMAIN,x.example,extra\nIP-CIDR,198.51.100.0/24,src\n",
        Format::Classical,
    );
    assert_eq!(
        reasons(&report),
        [
            ("line 4".to_owned(), Reason::Regex),
            ("line 7".to_owned(), Reason::Classical("IP-ASN".into())),
            (
                "line 8".to_owned(),
                Reason::Classical("PROCESS-NAME".into())
            ),
            ("line 9".to_owned(), Reason::Classical("DOMAIN".into())),
            ("line 10".to_owned(), Reason::Classical("IP-CIDR".into())),
        ]
    );
    assert_eq!(s.match_domain("exact.example"), Some(DomainHit::Exact));
    assert_eq!(
        s.match_domain("a.suffix.example"),
        Some(DomainHit::Suffix { len: 14 })
    );
    assert_eq!(
        s.match_domain("mytracker.example"),
        Some(DomainHit::Keyword { index: 0 })
    );
    assert!(s.match_ip(ip("192.0.2.9")) && s.match_ip(ip("2001:db8::9")));
    assert!(!s.match_ip(ip("198.51.100.1")));
}

#[test]
fn meshway_format_accepts_only_canonical_artifacts() {
    let original = convert(
        b"DOMAIN,a.example\nDOMAIN-SUFFIX,b.example\nDOMAIN-KEYWORD,kw\nIP-CIDR,10.0.0.1/32\nIP-CIDR,10.0.0.2/31\nIP-CIDR6,::/0\n",
        Format::Classical,
    )
    .unwrap();
    let again = convert(original.artifact.bytes(), Format::Meshway).unwrap();
    assert_eq!(again.artifact.digest(), original.artifact.digest());
    assert_eq!(again.report.summary.unconverted.len(), 0);
}

#[test]
fn report_items_are_bounded() {
    let mut source = String::new();
    for i in 0..1500 {
        source.push_str(&format!("bad_{i}.example\n"));
    }
    source.push_str("ok.example\n");
    let c = convert(source.as_bytes(), Format::MihomoDomainText).unwrap();
    assert_eq!(c.report.items.len(), 1000);
    assert_eq!(c.report.summary.skipped(), 1500);
    assert_eq!(c.report.summary.converted.exact, 1);
}
