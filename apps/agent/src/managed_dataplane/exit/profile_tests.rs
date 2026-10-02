use super::*;

fn catalog(default: &str, locked: bool) -> ProfileCatalog {
    ProfileCatalog {
        profiles: ["e", "f", "mesh"]
            .into_iter()
            .map(|id| RoutingProfile {
                id: id.into(),
                routes: vec![],
                direct_prefixes: vec![],
                fallback: if id == "mesh" {
                    ProfileFallback::Unmanaged
                } else {
                    ProfileFallback::Exit(ExitRoute {
                        prefix: "0.0.0.0/0".parse().unwrap(),
                        path: vec!["c".into(), id.into()],
                        return_path: vec![id.into(), "c".into()],
                    })
                },
            })
            .collect(),
        default_profile: default.into(),
        locked,
        automatic: Some(AutomaticProfile {
            region_to_profile: BTreeMap::from([
                ("US".into(), "e".into()),
                ("JP".into(), "f".into()),
            ]),
            resolved_profile: Some("e".into()),
            fallback_profile: "mesh".into(),
        }),
    }
}

fn plan(catalog: ProfileCatalog, cached: Option<&str>) -> ExitPlan {
    try_plan(catalog, cached).unwrap()
}

fn try_plan(catalog: ProfileCatalog, cached: Option<&str>) -> Result<ExitPlan, AgentError> {
    ExitPlan::new(
        "198.19.90.1".parse().unwrap(),
        "198.19.90.0/24".parse().unwrap(),
        1000,
        vec![],
        vec![],
    )
    .with_profiles(catalog, cached)
}

fn catalog_ids(ids: &[&str]) -> ProfileCatalog {
    ProfileCatalog {
        profiles: ids
            .iter()
            .map(|id| RoutingProfile {
                id: (*id).into(),
                routes: vec![],
                direct_prefixes: vec![],
                fallback: ProfileFallback::Unmanaged,
            })
            .collect(),
        default_profile: ids[0].into(),
        locked: false,
        automatic: None,
    }
}

#[test]
fn global_profile_keeps_full_identity_across_default_selection_and_automatic_resolution() {
    let identity = "0123456789abcdef".repeat(4);
    let global = format!("global:{identity}");
    assert_eq!(global.len(), 71);
    let mut c = catalog_ids(&[&global, "office.v1"]);
    c.profiles[0].fallback = ProfileFallback::Exit(ExitRoute {
        prefix: "0.0.0.0/0".parse().unwrap(),
        path: vec!["c".into(), identity.clone()],
        return_path: vec![identity.clone(), "c".into()],
    });
    let p = plan(c, None);
    let assert_global = |p: &ExitPlan, selected: &str| {
        assert_eq!(p.selected_profile.as_deref(), Some(selected));
        assert_eq!(p.effective_profile.as_deref(), Some(global.as_str()));
        assert_eq!(p.profiles.as_ref().unwrap().profiles[0].id, global);
        assert_eq!(
            p.route("203.0.113.9".parse().unwrap()).unwrap().path.last(),
            Some(&identity)
        );
    };
    assert_global(&p, &global);
    let custom = p.select_profile("office.v1").unwrap();
    assert_global(&custom.select_profile(&global).unwrap(), &global);
    assert!(
        p.select_profile(&format!("global:{}", &identity[..63]))
            .is_err()
    );

    let mut c = p.profiles.clone().unwrap();
    c.default_profile = "auto".into();
    c.automatic = Some(AutomaticProfile {
        region_to_profile: BTreeMap::from([("JP".into(), global.clone())]),
        resolved_profile: Some(global.clone()),
        fallback_profile: global.clone(),
    });
    let automatic = plan(c, None);
    assert_global(&automatic, "auto");
    assert_global(&automatic.resolve_automatic(None).unwrap(), "auto");
    assert_global(&automatic.resolve_automatic(Some(&global)).unwrap(), "auto");
    assert!(
        automatic
            .resolve_automatic(Some(&global.to_ascii_uppercase()))
            .is_err()
    );
}

#[test]
fn invalid_global_ids_duplicate_ids_and_auto_cannot_enter_concrete_catalog() {
    for invalid in [
        format!("global:{}", "a".repeat(63)),
        format!("global:{}", "a".repeat(65)),
        format!("global:{}", "A".repeat(64)),
        format!("global:{}", "g".repeat(64)),
        "global:*".into(),
        "auto".into(),
        "e".into(), // duplicate of the default
        String::new(),
        "office space".into(),
        "office/lan".into(),
        "office:lan".into(),
        "日本".into(),
        "x".repeat(65),
    ] {
        // Validate the entire allowed catalog, not only its chosen default.
        assert!(
            matches!(
                try_plan(catalog_ids(&["e", &invalid]), None),
                Err(AgentError::Invalid("invalid or duplicate routing profile"))
            ),
            "{invalid}"
        );
    }
    let global = format!("global:{}", "a".repeat(64));
    assert!(matches!(
        try_plan(catalog_ids(&[&global, &global]), None),
        Err(AgentError::Invalid("invalid or duplicate routing profile"))
    ));
}

#[test]
fn existing_custom_profile_ids_are_preserved_without_remapping() {
    let max_id = "x".repeat(64);
    for id in [
        "e",
        "f",
        "mesh",
        "global",
        "mesh-only",
        "Office.v1_lan-2",
        &max_id,
    ] {
        let p = plan(catalog_ids(&[id]), None);
        assert_eq!(p.selected_profile.as_deref(), Some(id));
        assert_eq!(p.effective_profile.as_deref(), Some(id));
        assert_eq!(p.profiles.as_ref().unwrap().profiles[0].id, id);
        assert_eq!(
            p.select_profile(id).unwrap().effective_profile.as_deref(),
            Some(id)
        );
    }
}

fn assert_exit(plan: &ExitPlan, selected: &str, effective: &str) {
    assert_eq!(plan.selected_profile.as_deref(), Some(selected));
    assert_eq!(plan.effective_profile.as_deref(), Some(effective));
    if effective == "mesh" {
        assert!(plan.route("203.0.113.9".parse().unwrap()).is_none());
    } else {
        assert_eq!(
            plan.route("203.0.113.9".parse().unwrap())
                .unwrap()
                .path
                .last()
                .unwrap(),
            effective
        );
    }
}

#[test]
fn locked_initialization_ignores_local_choice_and_only_default_is_selectable() {
    for cached in [
        None,
        Some("e"),
        Some("f"),
        Some("mesh"),
        Some("auto"),
        Some("removed"),
    ] {
        let p = plan(catalog("e", true), cached);
        assert_exit(&p, "e", "e");
        assert_exit(&p.select_profile("e").unwrap(), "e", "e");
        for requested in ["f", "mesh", "auto", "removed"] {
            assert!(p.select_profile(requested).is_err());
        }
    }
}

#[test]
fn locking_an_unlocked_selection_uses_controller_default() {
    let p = plan(catalog("e", false), Some("f"));
    assert_exit(&p, "f", "f");
    let cached = p.selected_profile.clone();
    let mut updated = p.profiles.clone().unwrap();
    updated.locked = true;
    let p = plan(updated, cached.as_deref());
    assert_exit(&p, "e", "e");
    assert!(p.select_profile("f").is_err());
}

#[test]
fn changed_locked_default_wins_over_cached_previous_default() {
    let p = plan(catalog("e", true), None);
    let mut updated = p.profiles.clone().unwrap();
    updated.default_profile = "f".into();
    let cached = p.selected_profile.clone();
    let p = plan(updated, cached.as_deref());
    assert_exit(&p, "f", "f");
    assert!(p.select_profile("e").is_err());
    assert_exit(&p.select_profile("f").unwrap(), "f", "f");
}

#[test]
fn locked_selection_compares_controller_default_not_current_selection() {
    let mut p = plan(catalog("e", true), None);
    // Exercise the selection boundary independently of initialization.
    p.selected_profile = Some("f".into());
    assert!(p.select_profile("f").is_err());
    assert_exit(&p.select_profile("e").unwrap(), "e", "e");
}

#[test]
fn locked_auto_allows_only_trusted_controller_resolution_updates() {
    let p = plan(catalog("auto", true), Some("f"));
    assert_exit(&p, "auto", "e");
    assert!(p.select_profile("e").is_err());
    assert!(p.select_profile("f").is_err());
    let p = p.resolve_automatic(Some("f")).unwrap();
    assert_exit(&p, "auto", "f");
    assert_exit(&p.select_profile("auto").unwrap(), "auto", "f");
    assert!(p.select_profile("f").is_err());
    assert!(p.resolve_automatic(Some("removed")).is_err());
    assert!(p.resolve_automatic(Some("auto")).is_err());
    assert_exit(&p.resolve_automatic(None).unwrap(), "auto", "mesh");
}
