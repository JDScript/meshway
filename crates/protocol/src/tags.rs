use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagDefinition {
    #[schema(pattern = "^tag:[a-z0-9][a-z0-9_-]{0,47}$")]
    pub tag: String,
    #[schema(max_length = 256)]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeTagAssignment {
    pub node_id: String,
    #[schema(max_items = 32)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagSnapshot {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(max_items = 64)]
    pub tags: Vec<TagDefinition>,
    pub assignments: Vec<NodeTagAssignment>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveTagsRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    #[schema(max_items = 64)]
    pub tags: Vec<TagDefinition>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetNodeTagsRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub node_id: String,
    #[schema(max_items = 32)]
    pub tags: Vec<String>,
}
