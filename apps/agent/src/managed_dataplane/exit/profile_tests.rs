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
    ExitPlan::new(
        "198.19.90.1".parse().unwrap(),
        "198.19.90.0/24".parse().unwrap(),
        1000,
        vec![],
        vec![],
    )
    .with_profiles(catalog, cached)
    .unwrap()
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
