use super::{
    crossing::Crossing,
    judge::{Interface, Truth},
};

/// Whether interface `actual` has the name `pattern`; a trailing `*` (nft)
/// or `+` (iptables) matches any suffix.
pub(super) fn interface(actual: Interface<'_>, pattern: &str, crossing: &Crossing) -> Truth {
    let present = &crossing.present;
    let (prefix, wildcard) = match pattern
        .strip_suffix('*')
        .or_else(|| pattern.strip_suffix('+'))
    {
        Some(prefix) => (prefix, true),
        None => (pattern, false),
    };
    let fresh = |name: &str| {
        name.strip_prefix("tun")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            && !present.iter().any(|known| known == name)
    };
    let truth = |value: bool| if value { Truth::Yes } else { Truth::No };
    match actual {
        Interface::Exit if let Some(name) = &crossing.installed => truth(if wildcard {
            name.starts_with(prefix)
        } else {
            name == pattern
        }),
        Interface::Named(name) if wildcard => truth(name.starts_with(prefix)),
        Interface::Named(name) => truth(name == pattern),
        Interface::Absent if wildcard => truth(prefix.is_empty()),
        Interface::Absent => truth(pattern.is_empty()),
        Interface::Exit if wildcard => {
            if "tun".starts_with(prefix) {
                Truth::Yes
            } else if prefix
                .strip_prefix("tun")
                .is_some_and(|number| number.bytes().all(|b| b.is_ascii_digit()))
            {
                Truth::Maybe
            } else {
                Truth::No
            }
        }
        Interface::Exit if fresh(pattern) => Truth::Maybe,
        Interface::Exit => Truth::No,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{crossing, name, nft, rule};
    use super::*;
    use meshway_model::exit::ExitUnavailableReason as Reason;
    use serde_json::json;

    #[test]
    fn exit_tun_names_are_fresh_kernel_tun_names() {
        let present = Crossing {
            present: vec!["tun0".to_owned(), "eth0".to_owned()],
            ..Default::default()
        };
        for (pattern, expected) in [
            ("tun*", Truth::Yes),
            ("t+", Truth::Yes),
            ("tun1*", Truth::Maybe),
            ("tun1", Truth::Maybe),
            ("tun0", Truth::No),
            ("tunnel*", Truth::No),
            ("eth0", Truth::No),
            ("*", Truth::Yes),
        ] {
            assert_eq!(
                interface(Interface::Exit, pattern, &present),
                expected,
                "{pattern}"
            );
        }
        assert_eq!(
            interface(Interface::Named("br-1"), "br-*", &present),
            Truth::Yes
        );
        assert_eq!(interface(Interface::Absent, "eth0", &present), Truth::No);
    }

    /// The periodic probe of an installed exit judges that exit's TUN by its
    /// name: a rule on `tun1` matches it, though `tun1` is present.
    #[test]
    fn an_installed_exit_tun_is_matched_by_its_name() {
        let mut crossing = crossing();
        crossing.present.push("tun1".into());
        let blocking = nft(json!([
            {"chain": {"family": "inet", "table": "own", "name": "forward", "type": "filter", "hook": "forward", "prio": 0, "policy": "accept"}},
            rule("inet", "own", "forward", json!([name("iifname", "==", json!("tun1")), {"drop": null}])),
        ]));
        assert_eq!(
            blocking.judge(&crossing),
            Ok(()),
            "a present tun1 is not a fresh exit TUN"
        );
        crossing.installed = Some("tun1".into());
        assert_eq!(blocking.judge(&crossing), Err(Reason::FirewallConflict));
        for (pattern, expected) in [
            ("tun1", Truth::Yes),
            ("tun*", Truth::Yes),
            ("tun0", Truth::No),
            ("tun1*", Truth::Yes),
            ("tun2", Truth::No),
        ] {
            assert_eq!(
                interface(Interface::Exit, pattern, &crossing),
                expected,
                "{pattern}"
            );
        }
    }
}
