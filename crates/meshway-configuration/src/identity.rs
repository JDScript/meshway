use crate::*;
use meshway_model::NodeId;
use std::collections::BTreeMap;

pub fn rebind_policy_user_references(
    c: &mut NetworkConfigurationInput,
    names: &BTreeMap<String, String>,
) {
    visit_policy(c, &mut |value| {
        if let Some(name) = names.get(value) {
            *value = name.clone();
        }
    });
}
pub fn rebind_node_user_references(
    c: &mut NetworkConfigurationInput,
    names: &BTreeMap<NodeId, String>,
) {
    for (id, node) in &mut c.nodes {
        if let NodeIdentity::User { user } = &mut node.identity
            && let Some(name) = names.get(id)
        {
            *user = name.clone();
        }
    }
}

/// Rename only identity/selector references. DNS, remarks, addresses, endpoint
/// names and protocol fields are never rewritten by string coincidence.
pub fn rename_user_references(c: &mut NetworkConfigurationInput, old: &str, new: &str) {
    visit(c, &mut |s| {
        if s == old {
            *s = new.into();
        }
    });
}

/// Simultaneous stable-binding substitutions, including exchanged email names.
/// The caller resolves all mappings by UserId AND MembershipId before calling.
pub fn rebind_user_references(c: &mut NetworkConfigurationInput, names: &BTreeMap<String, String>) {
    visit(c, &mut |s| {
        if let Some(name) = names.get(s) {
            *s = name.clone();
        }
    });
}
fn visit(c: &mut NetworkConfigurationInput, f: &mut impl FnMut(&mut String)) {
    for node in c.nodes.values_mut() {
        if let NodeIdentity::User { user } = &mut node.identity {
            f(user);
        }
    }
    visit_policy(c, f);
}
fn visit_policy(c: &mut NetworkConfigurationInput, f: &mut impl FnMut(&mut String)) {
    fn many(values: &mut [String], f: &mut impl FnMut(&mut String)) {
        for s in values {
            f(s);
        }
    }
    fn action(a: &mut RouteAction, f: &mut impl FnMut(&mut String)) {
        if let RouteAction::Exit { nodes, via } = a {
            many(nodes, f);
            many(via, f);
        }
    }
    fn rule_action(a: &mut RuleAction, f: &mut impl FnMut(&mut String)) {
        if let RuleAction::Exit { nodes, via } = a {
            many(nodes, f);
            many(via, f);
        }
    }
    for values in c.groups.values_mut().chain(c.tags_owners.values_mut()) {
        many(values, f);
    }
    for rule in &mut c.access {
        many(&mut rule.src, f);
        many(&mut rule.dst, f);
    }
    for rule in &mut c.topology {
        f(&mut rule.a);
        f(&mut rule.b);
    }
    for rule in &mut c.routing.mesh {
        many(&mut rule.src, f);
        many(&mut rule.dst, f);
        many(&mut rule.via, f);
    }
    for rule in &mut c.routing.egress.rules {
        rule_action(&mut rule.action, f);
    }
    for profile in c.routing.egress.profiles.values_mut() {
        action(&mut profile.fallback, f);
        for rule in &mut profile.rules {
            rule_action(&mut rule.action, f);
        }
    }
    for assignment in &mut c.routing.egress.assignments {
        many(&mut assignment.target, f);
    }
}
