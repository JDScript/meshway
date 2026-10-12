//! What each rule-set key of a configuration is made from, and its
//! conversion into an `MWRS` artifact through `meshway-rules` (the only
//! converter). Provenance and conversion reports stay outside the artifact.
use meshway_configuration::{
    RoutingConfiguration, RuleSet, RuleSetFormat, RuleSetProvider,
    providers::{effective_rule_set_providers, rule_set_reference},
};
use meshway_protocol::resources::{
    ResourceProblem, RuleSetConversion, RuleSetHeld, RuleSetSourceView,
};
use meshway_rules::{
    Artifact, ConvertError, Report,
    catalog::{Held, Location},
    convert::Format,
};
use std::{collections::BTreeMap, sync::Arc};

/// The source of one rule-set key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A `ruleSets` inline declaration: converted from the configuration.
    Inline(RuleSet),
    /// A `ruleSets` remote declaration.
    Remote(RuleSet),
    /// A `<provider>:<name>` reference: the set `<collection>/<name>` of the
    /// provider's signed catalog.
    Provider {
        provider: String,
        name: String,
        definition: RuleSetProvider,
    },
}

impl Source {
    /// The same content source: what is fetched and how it is converted. The
    /// refresh interval is not part of it.
    pub(crate) fn same_content(&self, other: &Source) -> bool {
        match (self, other) {
            (
                Self::Remote(RuleSet::Remote {
                    url: a, format: af, ..
                }),
                Self::Remote(RuleSet::Remote {
                    url: b, format: bf, ..
                }),
            ) => a == b && af == bf,
            (
                Self::Provider {
                    provider: ap,
                    name: an,
                    definition: a,
                },
                Self::Provider {
                    provider: bp,
                    name: bn,
                    definition: b,
                },
            ) => {
                ap == bp
                    && an == bn
                    && a.catalog == b.catalog
                    && a.collection == b.collection
                    && trust(&a.public_keys) == trust(&b.public_keys)
            }
            _ => self == other,
        }
    }

    pub(crate) fn needs_download(&self) -> bool {
        !matches!(self, Self::Inline(_))
    }

    /// Scheduled refresh period; `None` for inline content.
    pub(crate) fn interval_ms(&self) -> Option<u64> {
        match self {
            Self::Inline(_) => None,
            Self::Remote(RuleSet::Remote {
                update_interval, ..
            }) => Some(update_interval.as_millis()),
            Self::Remote(_) => None,
            Self::Provider { definition, .. } => Some(definition.update_interval.as_millis()),
        }
    }

    /// Identity of the content source for provenance records and download
    /// coalescing.
    pub(crate) fn identity(&self) -> String {
        match self {
            Self::Inline(_) => "inline".into(),
            Self::Remote(RuleSet::Remote { url, format, .. }) => {
                serde_json::json!({"remote": url, "format": format.as_str()}).to_string()
            }
            Self::Remote(_) => "inline".into(),
            Self::Provider {
                name, definition, ..
            } => serde_json::json!({
                "catalog": definition.catalog,
                "set": meshway_configuration::providers::catalog_set_name(definition, name),
                "trust": trust(&definition.public_keys),
            })
            .to_string(),
        }
    }

    /// The declaration shown in resource views; implicit sets have none.
    pub(crate) fn definition(&self) -> Option<&RuleSet> {
        match self {
            Self::Inline(set) | Self::Remote(set) => Some(set),
            Self::Provider { .. } => None,
        }
    }
}

/// Canonical spelling of a trusted key set.
pub(crate) fn trust(keys: &[String]) -> String {
    let mut keys = keys.to_vec();
    keys.sort();
    keys.dedup();
    keys.join(",")
}

/// Every rule-set key a configuration's egress rules can reference, with its
/// source: declared keys and every well-formed `<provider>:<name>` reference
/// of an enabled provider. Validation reports the others.
pub(crate) fn sources(
    rule_sets: &BTreeMap<String, RuleSet>,
    providers: &BTreeMap<String, Option<RuleSetProvider>>,
    routing: &RoutingConfiguration,
) -> BTreeMap<String, Source> {
    let mut out: BTreeMap<String, Source> = rule_sets
        .iter()
        .map(|(key, set)| {
            let source = match set {
                RuleSet::Inline { .. } => Source::Inline(set.clone()),
                RuleSet::Remote { .. } => Source::Remote(set.clone()),
            };
            (key.clone(), source)
        })
        .collect();
    let providers = effective_rule_set_providers(providers);
    let references = routing
        .egress
        .rules
        .iter()
        .chain(routing.egress.profiles.values().flat_map(|p| &p.rules))
        .flat_map(|rule| &rule.matcher.rule_sets);
    for key in references {
        let Some((provider, name)) = rule_set_reference(key) else {
            continue;
        };
        if let Some(definition) = providers.get(provider)
            && meshway_configuration::providers::valid_set_name(name)
        {
            out.entry(key.clone()).or_insert_with(|| Source::Provider {
                provider: provider.to_owned(),
                name: name.to_owned(),
                definition: definition.clone(),
            });
        }
    }
    out
}

/// JSON Pointer of a key's declaration, or of its first reference for an
/// implicit key: where its preparation problems are reported.
pub(crate) fn pointer(routing: &RoutingConfiguration, key: &str) -> String {
    let escape = |s: &str| s.replace('~', "~0").replace('/', "~1");
    if rule_set_reference(key).is_none() {
        return format!("/ruleSets/{}", escape(key));
    }
    let find = |rules: &[meshway_configuration::EgressRule], prefix: &str| {
        rules.iter().enumerate().find_map(|(i, rule)| {
            rule.matcher
                .rule_sets
                .iter()
                .position(|k| k == key)
                .map(|j| format!("{prefix}/{i}/match/ruleSets/{j}"))
        })
    };
    find(&routing.egress.rules, "/routing/egress/rules")
        .or_else(|| {
            routing.egress.profiles.iter().find_map(|(name, profile)| {
                find(
                    &profile.rules,
                    &format!("/routing/egress/profiles/{}/rules", escape(name)),
                )
            })
        })
        .unwrap_or_default()
}

/// Provenance of bound content, stored per (source, digest).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Record {
    /// Source format, or `inline`.
    pub format: String,
    pub upstream: Option<Location>,
    /// Sequence of the verified catalog the artifact was taken from.
    pub sequence: Option<u64>,
    pub held: Option<Held>,
    /// Complete for remote and inline sources; a catalog set carries its
    /// published report when it could be read, else its summary only.
    pub report: Report,
}

impl Record {
    pub(crate) fn conversion(&self) -> RuleSetConversion {
        (&self.report.summary).into()
    }
    pub(crate) fn source(&self) -> RuleSetSourceView {
        let upstream = self.upstream.as_ref();
        RuleSetSourceView {
            format: self.format.clone(),
            repo: upstream.map(|u| u.repo.clone()),
            reference: upstream.map(|u| u.reference.clone()),
            commit: upstream.map(|u| u.commit.clone()),
            path: upstream.map(|u| u.path.clone()),
        }
    }
    pub(crate) fn held(&self) -> Option<RuleSetHeld> {
        self.held.as_ref().map(|held| RuleSetHeld {
            since: held.since,
            reason: held.reason.clone(),
        })
    }
}

/// Content ready to bind: the artifact, its record and, for a catalog set,
/// the catalog sequence to accept in the binding transaction.
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub artifact: Artifact,
    pub record: Record,
    pub catalog: Option<CatalogAcceptance>,
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogAcceptance {
    pub catalog: String,
    pub trust: String,
    pub sequence: u64,
    /// The exact `index.json` and `index.json.sig` bytes that verified.
    pub document: Arc<[u8]>,
    pub signature: Arc<[u8]>,
}

fn problem(error: ConvertError) -> ResourceProblem {
    match error {
        ConvertError::TooLarge => ResourceProblem::TooLarge,
        ConvertError::InvalidSource(_) => ResourceProblem::InvalidSource,
        ConvertError::Budget(_) => ResourceProblem::RuleSetBudget,
        ConvertError::Empty => ResourceProblem::ConversionFailed,
    }
}

pub(crate) fn format(format: &RuleSetFormat) -> Format {
    match format {
        RuleSetFormat::SingBoxSource => Format::SingBoxSource,
        RuleSetFormat::MihomoDomain => Format::MihomoDomain,
        RuleSetFormat::MihomoIpcidr => Format::MihomoIpcidr,
        RuleSetFormat::MihomoDomainText => Format::MihomoDomainText,
        RuleSetFormat::MihomoIpcidrText => Format::MihomoIpcidrText,
        RuleSetFormat::Classical => Format::Classical,
    }
}

/// Converts downloaded source bytes; single entries never fail it.
pub(crate) fn convert(bytes: &[u8], source: &RuleSetFormat) -> Result<Prepared, ResourceProblem> {
    let conversion = meshway_rules::convert::convert(bytes, format(source)).map_err(problem)?;
    Ok(Prepared {
        artifact: conversion.artifact,
        record: Record {
            format: source.as_str().into(),
            upstream: None,
            sequence: None,
            held: None,
            report: conversion.report,
        },
        catalog: None,
    })
}

/// Converts an inline declaration. Validation rejects every invalid entry,
/// so a skipped entry here means the declaration cannot be bound.
pub(crate) fn inline(set: &RuleSet) -> Result<Prepared, ResourceProblem> {
    let RuleSet::Inline {
        ip_cidr,
        domain_suffix,
        domain,
        domain_subdomain,
    } = set
    else {
        return Err(ResourceProblem::InvalidSource);
    };
    let conversion = meshway_rules::inline::convert(&meshway_rules::inline::Inline {
        domain,
        domain_suffix,
        domain_subdomain,
        ip_cidr,
    })
    .map_err(problem)?;
    if !conversion.report.items.is_empty() {
        return Err(ResourceProblem::ConversionFailed);
    }
    Ok(Prepared {
        artifact: conversion.artifact,
        record: Record {
            format: "inline".into(),
            upstream: None,
            sequence: None,
            held: None,
            report: conversion.report,
        },
        catalog: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converter_reports_entries_and_keeps_exact_names() {
        let prepared = convert(
            b"payload:\n  - 'google.com'\n  - '+.google.com'\n  - '*.x.com'\n  - '.cdn.example'",
            &RuleSetFormat::MihomoDomain,
        )
        .unwrap();
        let set = prepared.artifact.clone().open().unwrap();
        assert!(set.match_domain("a.google.com").is_some());
        assert!(set.match_domain("a.cdn.example").is_some());
        assert!(set.match_domain("cdn.example").is_none());
        assert_eq!(prepared.record.report.summary.skipped(), 1);
        assert_eq!(
            convert(b"payload: [", &RuleSetFormat::MihomoDomain).unwrap_err(),
            ResourceProblem::InvalidSource
        );
        assert_eq!(
            convert(b"payload:\n  - '*.x.com'", &RuleSetFormat::MihomoDomain).unwrap_err(),
            ResourceProblem::ConversionFailed
        );
        let classical = convert(
            b"DOMAIN-SUFFIX,openai.com\nDOMAIN-KEYWORD,openai\nIP-ASN,1\n",
            &RuleSetFormat::Classical,
        )
        .unwrap();
        assert_eq!(classical.record.report.summary.skipped(), 1);
    }

    #[test]
    fn sources_expand_provider_references_and_honor_overrides() {
        let routing: RoutingConfiguration = serde_json::from_value(serde_json::json!({
            "egress": { "rules": [
                { "match": { "ruleSets": ["geosite:openai", "office", "geoip:cn", "nope:x"] },
                  "action": { "type": "direct" } }
            ] }
        }))
        .unwrap();
        let mut providers = BTreeMap::new();
        providers.insert("geoip".to_owned(), None);
        let sets = sources(&BTreeMap::new(), &providers, &routing);
        assert_eq!(sets.keys().collect::<Vec<_>>(), ["geosite:openai"]);
        let Source::Provider { definition, .. } = &sets["geosite:openai"] else {
            panic!()
        };
        assert_eq!(definition.collection, "meta-geosite");
        assert_eq!(
            meshway_configuration::providers::catalog_set_name(definition, "openai"),
            "meta-geosite/openai"
        );
        assert_eq!(
            pointer(&routing, "geosite:openai"),
            "/routing/egress/rules/0/match/ruleSets/0"
        );
    }
}
