use super::{Directory, Namespace, Reservation, SystemDnsError, storage, take_over};

fn reservation(inode: u64, cookie: Option<u64>) -> Reservation {
    Reservation {
        version: 1,
        namespace: Namespace {
            boot: "b".into(),
            device: 5,
            inode,
            cookie,
        },
        data_owner: "/var/lib/meshway".into(),
        settings: Default::default(),
    }
}

fn owner(directory: &Directory) -> Option<Reservation> {
    storage::read(directory, "owner.json").unwrap()
}

fn archives(directory: &Directory, record: &str) -> usize {
    let prefix = format!("{record}.stale-");
    directory
        .names()
        .unwrap()
        .iter()
        .filter(|name| name.starts_with(&prefix))
        .count()
}

/// The same inode under another cookie is a destroyed namespace: its
/// reservation and backend records are archived (the newest three kept,
/// as the Underlay keeps its journals) and the current one takes over.
#[test]
fn a_destroyed_namespaces_records_are_archived_and_taken_over() {
    let temp = tempfile::tempdir().unwrap();
    let directory = Directory::open(&temp.path().join("dns"), "dns.lock").unwrap();
    storage::save(&directory, "owner.json", &Some(&reservation(7, Some(8)))).unwrap();
    directory.replace("resolved.json", b"{}").unwrap();
    take_over(&directory, &reservation(7, Some(9))).unwrap();
    assert!(owner(&directory) == Some(reservation(7, Some(9))));
    assert!(
        !directory
            .names()
            .unwrap()
            .iter()
            .any(|name| name == "resolved.json")
    );
    assert_eq!(archives(&directory, "owner.json"), 1);
    assert_eq!(archives(&directory, "resolved.json"), 1);
    for cookie in 10..15 {
        directory.replace("resolv-conf.json", b"{}").unwrap();
        take_over(&directory, &reservation(7, Some(cookie))).unwrap();
        assert!(owner(&directory) == Some(reservation(7, Some(cookie))));
    }
    assert_eq!(archives(&directory, "owner.json"), 3);
    assert_eq!(archives(&directory, "resolv-conf.json"), 3);
}

/// Only the same namespace is the same owner: the same cookie with
/// another owner is a conflict, and so is a live process holding the
/// namespace's lock (it never reaches the records).
#[test]
fn the_same_namespace_with_another_owner_or_a_live_holder_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("dns");
    let directory = Directory::open(&path, "dns.lock").unwrap();
    storage::save(&directory, "owner.json", &Some(&reservation(7, Some(9)))).unwrap();
    take_over(&directory, &reservation(7, Some(9))).unwrap();
    let mut other = reservation(7, Some(9));
    other.data_owner = "/srv/other".into();
    assert!(matches!(
        take_over(&directory, &other),
        Err(SystemDnsError::Conflict)
    ));
    assert!(owner(&directory) == Some(reservation(7, Some(9))));
    assert_eq!(archives(&directory, "owner.json"), 0);
    assert!(
        Directory::open(&path, "dns.lock").is_err(),
        "the live holder's lock"
    );
    // A backend record without any reservation is never adopted.
    let bare = Directory::open(&temp.path().join("bare"), "dns.lock").unwrap();
    bare.replace("resolved.json", b"{}").unwrap();
    assert!(take_over(&bare, &reservation(7, Some(9))).is_err());
    assert!(owner(&bare).is_none());
}

/// A reservation an earlier build wrote without a cookie keeps the inode
/// rule: the same inode is taken over and rewritten with the cookie.
#[test]
fn a_reservation_without_a_cookie_follows_the_inode() {
    let temp = tempfile::tempdir().unwrap();
    let directory = Directory::open(&temp.path().join("dns"), "dns.lock").unwrap();
    storage::save(&directory, "owner.json", &Some(&reservation(7, None))).unwrap();
    take_over(&directory, &reservation(7, Some(9))).unwrap();
    assert!(owner(&directory) == Some(reservation(7, Some(9))));
    assert_eq!(archives(&directory, "owner.json"), 0);
    storage::save(&directory, "owner.json", &Some(&reservation(6, None))).unwrap();
    assert!(matches!(
        take_over(&directory, &reservation(7, Some(9))),
        Err(SystemDnsError::Conflict)
    ));
}

/// A reused inode with another cookie is another namespace; a record of
/// an earlier build (no cookie) or a kernel without cookies is compared
/// by the inode.
#[test]
fn namespaces_are_compared_by_cookie_when_both_carry_one() {
    let at = |inode, cookie| Namespace {
        boot: "b".into(),
        device: 5,
        inode,
        cookie,
    };
    assert!(at(7, Some(9)).same(&at(7, Some(9))));
    assert!(!at(7, Some(9)).same(&at(7, Some(8))), "reused inode");
    assert!(!at(7, Some(9)).same(&at(8, Some(9))));
    assert!(at(7, None).same(&at(7, Some(9))));
    assert!(at(7, Some(9)).same(&at(7, None)));
    assert!(!at(7, None).same(&at(8, None)));
    // An earlier build's record decodes without the field and writes none.
    let old: Namespace = serde_json::from_str(r#"{"boot":"b","device":5,"inode":7}"#).unwrap();
    assert_eq!(old, at(7, None));
    assert!(!serde_json::to_string(&old).unwrap().contains("cookie"));
    let current = Namespace::current().unwrap();
    assert!(current.cookie.is_some(), "Linux 5.7 or later");
    assert!(current.same(&Namespace::current().unwrap()));
}
