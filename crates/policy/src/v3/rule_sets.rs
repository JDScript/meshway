use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RuleSetEntry {
    DomainSuffix(String),
    Cidr(String),
    ExactDomain(String),
    Keyword(String),
    Regex(String),
    Unsupported(String),
}
/// Controller parsers translate provider-specific formats to these typed entries.
/// Unsupported predicates are retained for reporting, never approximated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ParsedRuleSet {
    pub entries: Vec<RuleSetEntry>,
}

pub fn expand_rule_sets(
    p: &Policy,
    sets: &BTreeMap<String, ParsedRuleSet>,
) -> Result<Policy, Vec<Diagnostic>> {
    if p.exits.len() > MAX_RULES
        || p.mesh.len() > MAX_RULES
        || p.grants.len() > MAX_RULES
        || p.connections.len() > MAX_RULES
        || p.defaults.len() > MAX_RULES
        || serde_json::to_vec(p).map_or(true, |v| v.len() > 2 * 1024 * 1024)
    {
        return Err(vec![Diagnostic {
            field: "policy".into(),
            message: "Policy exceeds rule/2 MiB source bounds".into(),
        }]);
    }
    let mut out = p.clone();
    let mut diagnostics = Vec::new();
    let mut total = 0;
    for (i, rule) in out.exits.iter_mut().enumerate() {
        let mut destinations = Vec::new();
        for (j, d) in rule.destinations.iter().enumerate() {
            if let ExitMatch::RuleSet(id) = d {
                let field = format!("exits[{i}].destinations[{j}]");
                if id.is_empty()
                    || id.len() > 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    diagnostics.push(Diagnostic {
                        field,
                        message: "Rule-set reference requires 1..64 ASCII identifier".into(),
                    });
                    continue;
                }
                let Some(set) = sets.get(id) else {
                    diagnostics.push(Diagnostic {
                        field,
                        message: format!("Rule set {id} missing from coherent controller snapshot"),
                    });
                    continue;
                };
                if set.entries.len() > MAX_DESTINATIONS {
                    return Err(vec![Diagnostic {
                        field,
                        message: "Parsed rule set exceeds 16384 entries".into(),
                    }]);
                }
                if set.entries.is_empty() {
                    diagnostics.push(Diagnostic {
                        field: field.clone(),
                        message: format!("Rule set {id} empty/unavailable"),
                    });
                }
                for (k, e) in set.entries.iter().enumerate() {
                    if total >= MAX_DESTINATIONS {
                        return Err(vec![Diagnostic {
                            field,
                            message: "Expanded destinations exceed 16384".into(),
                        }]);
                    }
                    let value = match e {
                        RuleSetEntry::DomainSuffix(s) => meshway_protocol::normalize_dns_name(s)
                            .map(ExitMatch::DomainSuffix)
                            .map_err(str::to_owned),
                        RuleSetEntry::Cidr(s) => s
                            .parse::<ipnet::IpNet>()
                            .map(|n| ExitMatch::Cidr(n.trunc().to_string()))
                            .map_err(|_| "Invalid CIDR".into()),
                        _ => Err("Unsupported exact-domain/keyword/regex/provider predicate cannot be exactly represented by suffix/CIDR".into()),
                    };
                    match value {
                        Ok(d) => {
                            destinations.push(d);
                            total += 1;
                        }
                        Err(message) => {
                            if diagnostics.len() < 128 {
                                diagnostics.push(Diagnostic {
                                    field: field.clone(),
                                    message: format!("rule_sets[{id}].entries[{k}]: {message}"),
                                });
                            }
                        }
                    }
                }
            } else {
                destinations.push(d.clone());
                total += 1;
            }
            if total > MAX_DESTINATIONS {
                return Err(vec![Diagnostic {
                    field: format!("exits[{i}].destinations[{j}]"),
                    message: "Expanded destinations exceed 16384".into(),
                }]);
            }
        }
        destinations.sort();
        destinations.dedup();
        rule.destinations = destinations;
    }
    if diagnostics.is_empty() {
        Ok(out)
    } else {
        Err(diagnostics)
    }
}
