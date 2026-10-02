use meshway_policy::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn rejects<T: DeserializeOwned>(raw: Value) {
    assert!(serde_json::from_value::<T>(raw.clone()).is_err(), "{raw}");
}

#[test]
fn fallback_rejects_unknown_fields_at_every_source_boundary() {
    for fallback in [
        json!({"kind":"none", "typo":true}),
        json!({"kind":"none", "candidates":[]}),
        json!({"kind":"none", "via":[]}),
        json!({"kind":"direct", "candidates":["unexpected"]}),
        json!({"kind":"direct", "via":[]}),
        json!({"kind":"nodes", "candidates":[], "via":[], "typo":true}),
        json!({"kind":"nodes", "via":[]}),
        json!({"kind":"nodes", "candidates":[]}),
    ] {
        rejects::<ProfileFallback>(fallback.clone());
        let profile = json!({"id":"office", "rules":[], "fallback":fallback});
        rejects::<RoutingProfile>(profile.clone());
        let mut network = serde_json::to_value(NetworkPolicy::default_template()).unwrap();
        network["profiles"] = json!([profile]);
        rejects::<NetworkPolicy>(network);
    }
}

#[test]
fn adjacent_tagged_policy_boundaries_reject_extra_fields_and_invalid_content() {
    for kind in ["none", "direct", "nodes"] {
        let mut final_value = json!({"kind":kind, "typo":true});
        if kind == "nodes" {
            final_value["paths"] = json!([0]);
        }
        rejects::<CompiledFinal>(final_value);
    }
    for kind in ["none", "direct"] {
        rejects::<CompiledFinal>(json!({"kind":kind, "paths":[0]}));
    }
    for kind in ["domain_suffix", "cidr", "rule_set"] {
        rejects::<ExitMatch>(json!({"kind":kind, "value":"example", "typo":true}));
        rejects::<ExitMatch>(json!({"kind":kind}));
    }
    for kind in [
        "domain_suffix",
        "cidr",
        "exact_domain",
        "keyword",
        "regex",
        "unsupported",
    ] {
        rejects::<RuleSetEntry>(json!({"kind":kind, "value":"example", "typo":true}));
        rejects::<RuleSetEntry>(json!({"kind":kind}));
    }
}

#[test]
fn fallback_wire_shape_and_derived_schema_stay_closed_and_compatible() {
    let schema =
        serde_json::to_value(<ProfileFallback as utoipa::PartialSchema>::schema()).unwrap();
    let branches = schema["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 3);
    for (fallback, wire) in [
        (ProfileFallback::None {}, json!({"kind":"none"})),
        (ProfileFallback::Direct {}, json!({"kind":"direct"})),
        (
            ProfileFallback::Nodes {
                candidates: vec!["tag:exit".into()],
                via: vec![],
            },
            json!({"kind":"nodes", "candidates":["tag:exit"], "via":[]}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&fallback).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ProfileFallback>(wire.clone()).unwrap(),
            fallback
        );
        let branch = branches
            .iter()
            .find(|b| b["properties"]["kind"]["enum"] == json!([wire["kind"]]))
            .unwrap();
        // Every declared property is required; this exact bound rejects any
        // extra key even where utoipa omits additionalProperties for variants.
        assert_eq!(branch["maxProperties"], wire.as_object().unwrap().len());
        assert_eq!(
            branch["properties"].as_object().unwrap().len(),
            wire.as_object().unwrap().len()
        );
        let required = branch["required"].as_array().unwrap();
        for key in wire.as_object().unwrap().keys() {
            assert!(required.contains(&json!(key)));
        }
    }
}
