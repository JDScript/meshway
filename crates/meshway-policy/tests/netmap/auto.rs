//! `auto` resolved by the source's declared region: every plan, decision and
//! selection identity uses the resolved profile, and relays and exits
//! recompute the same resolution from their projections.
use super::*;
use meshway_configuration::selection::AutoBasis;
use std::net::IpAddr;

/// Source 1 may reach the Internet on tcp:443; `auto` sends US to `exit`
/// (exit 4 via 3) and JP to `alt` (exit 5), anything else to `mesh`.
fn scenario(region: Option<&str>) -> Scenario {
    let mut s = base(5);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    for (name, exit, via) in [("exit", 4, vec![sel(3)]), ("alt", 5, vec![])] {
        s.cfg.routing.egress.profiles.insert(
            name.into(),
            RoutingProfile {
                rules: vec![],
                fallback: RouteAction::Exit {
                    nodes: vec![sel(exit)],
                    via,
                },
            },
        );
    }
    s.cfg.routing.egress.auto = Input::Value(AutoProfile {
        rules: vec![
            CountryProfile {
                countries: vec!["US".into()],
                profile: "exit".into(),
            },
            CountryProfile {
                countries: vec!["JP".into()],
                profile: "alt".into(),
            },
        ],
        fallback: "mesh".into(),
    });
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["auto".into(), "exit".into(), "alt".into(), "mesh".into()],
        default: "auto".into(),
        locked: false,
    });
    s.cfg.nodes.get_mut(&node(1)).unwrap().region = region.map(str::to_owned);
    s
}

fn compiler(compiled: &CompiledNetmaps, at: u8) -> LocalCompiler {
    LocalCompiler::new(&compiled.nodes[&node(at)].netmap, &Default::default())
}

fn generation(compiled: &CompiledNetmaps) -> Digest {
    compiled.nodes[&node(1)]
        .netmap
        .egress
        .as_ref()
        .unwrap()
        .generation
}

fn resolution(compiled: &CompiledNetmaps) -> (String, AutoBasis, Option<String>) {
    let state =
        meshway_policy::compile::netmap::routing_selection(&compiled.nodes[&node(1)].netmap)
            .unwrap();
    assert_eq!(state.applied_profile, "auto");
    let auto = state.auto.unwrap();
    (auto.profile, auto.basis, auto.region)
}

/// The exit `decide` sends 1.1.1.1:443 to, or None when it is not an exit flow.
fn exit_for(compiled: &CompiledNetmaps) -> Option<NodeId> {
    let destination: IpAddr = "1.1.1.1".parse().unwrap();
    match compiler(compiled, 1)
        .decide(node(1), destination, BusinessProtocol::Tcp, port(443))
        .unwrap()
    {
        LocalDecision::Permitted(decision) => {
            let exits: Vec<_> = decision
                .paths
                .iter()
                .map(|p| *p.nodes.last().unwrap())
                .collect();
            assert!(exits.windows(2).all(|w| w[0] == w[1]));
            // Every Node on the path recomputes the source's identity.
            for hop in decision.paths.iter().flat_map(|p| p.nodes.iter().skip(1)) {
                let at = LocalCompiler::new(&compiled.nodes[hop].netmap, &Default::default());
                assert_eq!(at.egress(node(1)).unwrap().generation, generation(compiled));
            }
            exits.first().copied()
        }
        _ => None,
    }
}

#[test]
fn the_declared_region_selects_the_first_listing_rule_and_otherwise_the_fallback() {
    let none = netmaps(&scenario(None));
    assert_eq!(
        resolution(&none),
        ("mesh".into(), AutoBasis::NoTrustedRegion, None)
    );
    assert_eq!(exit_for(&none), None);

    let us = netmaps(&scenario(Some("US")));
    assert_eq!(
        resolution(&us),
        ("exit".into(), AutoBasis::Label, Some("US".into()))
    );
    assert_eq!(exit_for(&us), Some(node(4)));
    assert_eq!(
        us.nodes[&node(1)].netmap.egress.as_ref().unwrap().fallback,
        EgressAction::Exit {
            nodes: vec![node(4)],
            via: vec![node(3)],
        }
    );

    let jp = netmaps(&scenario(Some("JP")));
    assert_eq!(
        resolution(&jp),
        ("alt".into(), AutoBasis::Label, Some("JP".into()))
    );
    assert_eq!(exit_for(&jp), Some(node(5)));

    let fr = netmaps(&scenario(Some("FR")));
    assert_eq!(
        resolution(&fr),
        ("mesh".into(), AutoBasis::NoRuleMatched, Some("FR".into()))
    );
    assert_eq!(exit_for(&fr), None);
}

#[test]
fn every_change_of_the_resolution_changes_the_selection_identity() {
    let regions = [None, Some("US"), Some("JP"), Some("FR"), Some("DE")];
    let generations: Vec<_> = regions
        .iter()
        .map(|region| generation(&netmaps(&scenario(*region))))
        .collect();
    for (i, a) in generations.iter().enumerate() {
        for (j, b) in generations.iter().enumerate() {
            // FR and DE both fall back for different regions: still distinct.
            assert_eq!(a == b, i == j, "{:?} {:?}", regions[i], regions[j]);
        }
    }
    // The same resolution is the same identity (a switch back included).
    assert_eq!(generation(&netmaps(&scenario(Some("US")))), generations[1]);
    // An explicit selection of the same concrete profile is another identity:
    // its selection version is part of it.
    let mut selected = scenario(Some("US"));
    selected
        .selection_versions
        .insert(node(1), Version::new(1).unwrap());
    let explicit = compile_netmap(NetmapInput {
        network: network(),
        revision: Revision::MIN,
        configuration: &selected.cfg,
        subjects: &selected.subjects,
        inventory: &selected.inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::from([(node(1), "exit".to_owned())]),
        selection_versions: &selected.selection_versions,
        previous: &BTreeMap::new(),
    })
    .unwrap();
    let state =
        meshway_policy::compile::netmap::routing_selection(&explicit.nodes[&node(1)].netmap)
            .unwrap();
    assert_eq!((state.applied_profile.as_str(), state.auto), ("exit", None));
    assert_ne!(generation(&explicit), generations[1]);
    assert_eq!(exit_for(&explicit), Some(node(4)));
}

#[test]
fn the_region_is_projected_to_the_exit_and_relay_of_the_source() {
    let us = netmaps(&scenario(Some("US")));
    for at in [3, 4] {
        let view = &us.nodes[&node(at)].netmap.projection.configuration.nodes[&node(1)];
        assert_eq!(view.region.as_deref(), Some("US"), "at {at}");
    }
}
