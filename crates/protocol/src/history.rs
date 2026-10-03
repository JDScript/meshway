use crate::object::{wire_object, wire_tagged};
use chrono::{DateTime, Utc};
use meshway_model::{
    ActivityId, JoinLinkId, MembershipId, NetworkId, RequestId, Revision, SessionId, UserId,
};
fn nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    serde::Deserialize::deserialize(d)
}

wire_tagged! {pub enum ActivityActor {User{#[serde(rename="userId")]user_id:UserId},System{}}}
wire_tagged! {pub enum ActivityTarget {Account{#[serde(rename="userId")]user_id:UserId},Session{#[serde(rename="sessionId")]session_id:SessionId},Network{#[serde(rename="networkId")]network_id:NetworkId},Membership{#[serde(rename="membershipId")]membership_id:MembershipId},JoinLink{#[serde(rename="linkId")]link_id:JoinLinkId},Configuration{revision:Revision}}}

macro_rules! actions {
    ($($variant:ident = $wire:literal),* $(,)?) => {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
        #[serde(try_from = "String")]
        pub enum ActivityAction { $(#[serde(rename = $wire)] $variant),* }
        impl TryFrom<String> for ActivityAction {
            type Error = &'static str;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                match value.as_str() {
                    $($wire => Ok(Self::$variant),)*
                    _ => Err("unknown activity action"),
                }
            }
        }
        impl utoipa::PartialSchema for ActivityAction {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                use utoipa::openapi::schema::{ObjectBuilder, Type};
                // The wire spelling is the authority for both Serde and Schema;
                // schema derives do not reliably see macro-interpolated renames.
                ObjectBuilder::new().schema_type(Type::String)
                    .enum_values(Some([$($wire),*])).into()
            }
        }
        impl utoipa::ToSchema for ActivityAction {}
    };
}
actions! {
    AccountRegister = "account.register",
    SessionLogin = "session.login",
    SessionLogout = "session.logout",
    SessionRevoke = "session.revoke",
    SessionRevokeOthers = "session.revokeOthers",
    SessionRevokeAll = "session.revokeAll",
    AccountPassword = "account.password",
    AccountPasswordReset = "account.passwordReset",
    AccountResetIssued = "account.resetIssued",
    AccountEmail = "account.email",
    NetworkCreate = "network.create",
    NetworkRename = "network.rename",
    ConfigurationApply = "configuration.apply",
    MembershipRole = "membership.role",
    MembershipJoin = "membership.join",
    MembershipRemove = "membership.remove",
    JoinLinkCreate = "joinLink.create",
    JoinLinkRevoke = "joinLink.revoke",
}

wire_object! {pub struct ActivityEvent {
    pub id:ActivityId,pub at:DateTime<Utc>,pub actor:ActivityActor,pub action:ActivityAction,pub target:ActivityTarget,
    #[serde(deserialize_with="nullable")]pub before_revision:Option<Revision> [required=true],
    #[serde(deserialize_with="nullable")]pub after_revision:Option<Revision> [required=true],
    pub request_id:RequestId,
} }
wire_object! {pub struct ActivityListResponse {pub request_id:RequestId,pub items:Vec<ActivityEvent>,#[serde(deserialize_with="nullable")]pub next_cursor:Option<String> [required=true],pub as_of:DateTime<Utc>} }
wire_object! {pub struct ActivityResponse {pub request_id:RequestId,pub activity:ActivityEvent} }

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum ConfigurationChangeCause {
    NetworkCreated,
    Edit,
    Restore,
    Enrollment,
    NodeDeleted,
    EmailSync,
    MemberRemoved,
    IdentityTransition,
}
impl ConfigurationChangeCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NetworkCreated => "networkCreated",
            Self::Edit => "edit",
            Self::Restore => "restore",
            Self::Enrollment => "enrollment",
            Self::NodeDeleted => "nodeDeleted",
            Self::EmailSync => "emailSync",
            Self::MemberRemoved => "memberRemoved",
            Self::IdentityTransition => "identityTransition",
        }
    }
}
impl TryFrom<String> for ConfigurationChangeCause {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "networkCreated" => Ok(Self::NetworkCreated),
            "edit" => Ok(Self::Edit),
            "restore" => Ok(Self::Restore),
            "enrollment" => Ok(Self::Enrollment),
            "nodeDeleted" => Ok(Self::NodeDeleted),
            "emailSync" => Ok(Self::EmailSync),
            "memberRemoved" => Ok(Self::MemberRemoved),
            "identityTransition" => Ok(Self::IdentityTransition),
            _ => Err("unknown change cause"),
        }
    }
}
wire_object! {pub struct ConfigurationRevisionSummary {pub revision:Revision,pub created_at:DateTime<Utc>,pub created_by:ActivityActor,pub cause:ConfigurationChangeCause,pub current:bool,#[serde(deserialize_with="nullable")]pub rollback_from:Option<Revision> [required=true]} }
wire_object! {pub struct ConfigurationRevisionList {pub request_id:RequestId,pub current_revision:Revision,pub items:Vec<ConfigurationRevisionSummary>,#[serde(deserialize_with="nullable")]pub next_cursor:Option<String> [required=true],pub as_of:DateTime<Utc>} }
