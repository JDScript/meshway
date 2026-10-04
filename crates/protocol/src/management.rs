use crate::{UserView, WireSecret, object::wire_object};
use chrono::{DateTime, Utc};
use meshway_model::{
    JoinLinkId, MembershipId, MembershipRole, NetworkId, NodeId, RequestId, Revision, SessionId,
    UserId, Version,
};

fn nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    serde::Deserialize::deserialize(d)
}

wire_object! { pub struct AccountResponse { pub request_id: RequestId, pub user: UserView } }
wire_object! { pub struct ChangePasswordRequest { pub current_password: WireSecret, pub password: WireSecret } }
wire_object! { pub struct ChangeEmailRequest { pub password: WireSecret, pub email: String } }
wire_object! { pub struct PasswordResetInspectRequest { pub token: WireSecret } }
wire_object! { pub struct PasswordResetRedeemRequest { pub token: WireSecret, pub password: WireSecret } }
wire_object! { pub struct PasswordResetInspection { pub request_id: RequestId, pub expires_at: DateTime<Utc> } }
wire_object! { pub struct AccountSessionView {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(deserialize_with="nullable")] pub last_used_at: Option<DateTime<Utc>> [required=true],
    #[serde(deserialize_with="nullable")] pub client_label: Option<String> [required=true],
    pub current: bool,
} }
wire_object! { pub struct AccountSessionsResponse {
    pub request_id: RequestId, pub items: Vec<AccountSessionView>,
    #[serde(deserialize_with="nullable")] pub next_cursor: Option<String> [required=true],
    pub as_of: DateTime<Utc>,
} }

wire_object! { pub struct ChangeMemberRoleRequest { pub expected_version: Version, pub role: MembershipRole } }
wire_object! { pub struct RemoveMemberRequest { pub expected_version:Version,pub expected_revision:Revision } }
wire_object! { pub struct RemovalImpactResponse { pub request_id:RequestId,pub membership_id:MembershipId,pub version:Version,pub revision:Revision,pub node_ids:Vec<NodeId>,pub node_count:u32 } }
wire_object! { pub struct MemberRemovedResponse { pub request_id:RequestId,pub revision:Revision } }
wire_object! { pub struct MemberMutationResponse { pub request_id: RequestId, pub member: crate::MemberSummary } }
wire_object! { pub struct CreateJoinLinkRequest { pub request_id: RequestId, pub role: MembershipRole, pub expires_in_seconds: u32 [minimum=60,maximum=604800] } }
wire_object! { pub struct JoinLinkCredentialRequest { pub token: WireSecret } }
wire_object! { #[derive(Debug,Clone)] pub struct JoinLinkView {
    pub id: JoinLinkId, pub network_id: NetworkId, pub role: MembershipRole,
    pub created_at: DateTime<Utc>,pub expires_at: DateTime<Utc>,
    #[serde(deserialize_with="nullable")] pub revoked_at: Option<DateTime<Utc>> [required=true],
    #[serde(deserialize_with="nullable")] pub consumed_at: Option<DateTime<Utc>> [required=true],
    #[serde(deserialize_with="nullable")] pub consumed_by_user_id: Option<UserId> [required=true],
    pub actions: JoinLinkActions,
} }
wire_object! { #[derive(Debug,Clone)] pub struct JoinLinkActions { pub revoke: bool } }
wire_object! { pub struct JoinLinkResponse { pub request_id: RequestId, pub link: JoinLinkView } }
wire_object! { pub struct JoinLinkInspection { pub request_id: RequestId, pub link: JoinLinkView, pub network_name: String, pub already_member: bool } }
wire_object! { pub struct JoinLinkListResponse {
    pub request_id:RequestId,pub items:Vec<JoinLinkView>,
    #[serde(deserialize_with="nullable")] pub next_cursor:Option<String> [required=true],
    pub as_of:DateTime<Utc>,
    pub total:u64,
} }
wire_object! { pub struct JoinLinkRedeemed { pub request_id:RequestId,pub network_id:NetworkId,pub membership_id:MembershipId } }

// Separate branches make unavailable delivery impossible to mistake for a
// recoverable original. Lists and ordinary reads never carry this type.
crate::object::wire_tagged! {tag="secretDelivery";pub enum JoinLinkCreated {
    Available {
        #[serde(rename = "requestId")]
        request_id: RequestId,
        link: JoinLinkView,
        secret: WireSecret,
    },
    Unavailable {
        #[serde(rename = "requestId")]
        request_id: RequestId,
        link: JoinLinkView,
    },
} }
