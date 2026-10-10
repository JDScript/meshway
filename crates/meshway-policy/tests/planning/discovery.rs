use super::*;

#[test]
fn discovery_wildcards_and_node_selectors_cannot_bypass_active_identity() {
    let mut c = configuration();
    let binding = UserBinding {
        user_id: "00000000-0000-4006-8000-000000000001".parse().unwrap(),
        membership_id: "00000000-0000-4006-8000-000000000002".parse().unwrap(),
    };
    let email = "member@example.test".to_owned();
    c.nodes.get_mut(&node(2)).unwrap().identity = NodeIdentity::User {
        user: email.clone(),
    };
    let mut subjects = SubjectContext {
        users: BTreeMap::from([(email.clone(), binding.clone())]),
        node_users: BTreeMap::from([(node(2), binding.clone())]),
        members: BTreeMap::from([(email.clone(), binding.clone())]),
    };
    for selector in ["*".to_owned(), format!("node:{}", node(2))] {
        c.access[0].dst = vec![selector];
        assert!(mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        subjects.members.clear();
        assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        let mut replacement = binding.clone();
        replacement.membership_id = "00000000-0000-4006-8000-000000000003".parse().unwrap();
        subjects.members.insert(email.clone(), replacement);
        assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        subjects.members.insert(email.clone(), binding.clone());
    }
    c.tags_owners.clear();
    assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
}
