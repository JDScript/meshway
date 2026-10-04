//! Trusted stable resolution, separate from editable selectors and Node claims.
use meshway_configuration::{NetworkConfiguration, NodeIdentity, UserBinding};
use meshway_model::NodeId;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SubjectContext {
    pub users: BTreeMap<String, UserBinding>,
    pub node_users: BTreeMap<NodeId, UserBinding>,
    /// Current active memberships, loaded in the same Controller transaction.
    pub members: BTreeMap<String, UserBinding>,
}
impl SubjectContext {
    pub fn node_is_active(&self, config: &NetworkConfiguration, id: NodeId) -> bool {
        config
            .nodes
            .get(&id)
            .is_some_and(|node| match &node.identity {
                NodeIdentity::User { user } => self
                    .node_users
                    .get(&id)
                    .is_some_and(|binding| self.members.get(user) == Some(binding)),
                NodeIdentity::Tag { tags } => {
                    !self.node_users.contains_key(&id)
                        && !tags.is_empty()
                        && tags.iter().all(|tag| config.tags_owners.contains_key(tag))
                }
            })
    }
    pub fn matches(&self, config: &NetworkConfiguration, selector: &str, id: NodeId) -> bool {
        let Some(node) = config.nodes.get(&id) else {
            return false;
        };
        if selector == "*"
            || selector
                .strip_prefix("node:")
                .is_some_and(|value| value.parse() == Ok(id))
        {
            return true;
        }
        match &node.identity {
            NodeIdentity::Tag { tags } => tags.iter().any(|tag| tag == selector),
            NodeIdentity::User { user } => {
                let Some(binding) = self
                    .node_users
                    .get(&id)
                    .filter(|binding| self.members.get(user) == Some(*binding))
                else {
                    return false;
                };
                let matches = |name: &String| self.users.get(name) == Some(binding);
                self.users.get(selector) == Some(binding)
                    || config
                        .groups
                        .get(selector)
                        .is_some_and(|users| users.iter().any(matches))
            }
        }
    }
}
