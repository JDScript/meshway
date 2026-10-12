//! Rule-set catalog providers: the built-in definitions and the effective set
//! a network configuration selects (`ruleSetProviders` replaces a built-in
//! provider of the same name as a whole; `null` disables it).
use crate::RuleSetProvider;
use std::collections::BTreeMap;

/// The published catalog of `meshway-org/meshway-rules` (`dist` branch).
pub const BUILTIN_CATALOG: &str =
    "https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json";

/// The trusted catalog key of the built-in providers (the publishing
/// workflow's key). Rotating it is a Server release pinning the new key.
pub const BUILTIN_PUBLIC_KEYS: [&str; 1] =
    ["ed25519:68dfc59cf585e0320129db7f1716b0dbb3d13c462fcefee7151d4b51ae1fc45c"];

/// `geosite:<category>` and `geoip:<country>`, aligned with mihomo's
/// meta-rules-dat collections.
pub const BUILTIN_PROVIDERS: [(&str, &str); 2] =
    [("geosite", "meta-geosite"), ("geoip", "meta-geoip")];

pub fn builtin_rule_set_providers() -> BTreeMap<String, RuleSetProvider> {
    BUILTIN_PROVIDERS
        .iter()
        .map(|(name, collection)| {
            (
                (*name).to_owned(),
                RuleSetProvider {
                    catalog: BUILTIN_CATALOG.to_owned(),
                    collection: (*collection).to_owned(),
                    public_keys: BUILTIN_PUBLIC_KEYS
                        .iter()
                        .map(|k| (*k).to_owned())
                        .collect(),
                    update_interval: "24h".parse().expect("constant duration"),
                },
            )
        })
        .collect()
}

/// Built-in providers with the configuration's replacements applied.
pub fn effective_rule_set_providers(
    configured: &BTreeMap<String, Option<RuleSetProvider>>,
) -> BTreeMap<String, RuleSetProvider> {
    let mut providers = builtin_rule_set_providers();
    for (name, provider) in configured {
        match provider {
            Some(provider) => {
                providers.insert(name.clone(), provider.clone());
            }
            None => {
                providers.remove(name);
            }
        }
    }
    providers
}

/// `<provider>:<name>` split at the first `:`; `None` for a network key.
pub fn rule_set_reference(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

/// The catalog set a `<provider>:<name>` reference selects.
pub fn catalog_set_name(provider: &RuleSetProvider, name: &str) -> String {
    format!("{}/{name}", provider.collection)
}

/// Provider names: `[a-z][a-z0-9-]{0,31}`.
pub fn valid_provider_name(name: &str) -> bool {
    name.len() <= 32
        && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Catalog collections: `[a-z0-9][a-z0-9-]{0,31}`.
pub fn valid_collection(collection: &str) -> bool {
    collection.len() <= 32
        && collection
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && collection
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Set names within a collection: `[a-z0-9][a-z0-9!@._+-]{0,127}`.
pub fn valid_set_name(name: &str) -> bool {
    name.len() <= 128
        && name
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!@._+-".contains(&b))
}

/// `ed25519:` and 64 lowercase hex digits.
pub fn valid_public_key(key: &str) -> bool {
    key.strip_prefix("ed25519:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_is_whole_and_null_disables() {
        let mut configured = BTreeMap::new();
        configured.insert("geoip".to_owned(), None);
        let corp = RuleSetProvider {
            catalog: "https://rules.example/index.json".into(),
            collection: "corp".into(),
            public_keys: vec![format!("ed25519:{}", "1".repeat(64))],
            update_interval: "1h".parse().unwrap(),
        };
        configured.insert("geosite".to_owned(), Some(corp.clone()));
        let effective = effective_rule_set_providers(&configured);
        assert_eq!(effective.len(), 1);
        assert_eq!(effective["geosite"], corp);
        assert_eq!(
            rule_set_reference("geosite:openai"),
            Some(("geosite", "openai"))
        );
        assert_eq!(rule_set_reference("office"), None);
        assert!(BUILTIN_PUBLIC_KEYS.iter().all(|k| valid_public_key(k)));
        assert!(valid_set_name("geolocation-!cn") && !valid_set_name("-x"));
    }
}
