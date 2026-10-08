use crate::object::{wire_object, wire_tagged};
use chrono::{DateTime, Utc};
use meshway_model::{
    ActivityId, JoinLinkId, MembershipId, NetworkId, RequestId, Revision, SessionId, UserId,
};
use meshway_serde::required_nullable as nullable;

wire_tagged! {
/// Verified user or system responsible for a committed activity.
pub enum ActivityActor {
/// User identity with an email snapshot from the time of the activity.
User{
    /// Stable identifier of the user who performed the action.
    #[serde(rename="userId")]user_id:UserId,
    /// Email at the time of the event; null means no historical snapshot exists.
    #[serde(deserialize_with="nullable")]email:Option<String> [required=true],
},
/// Server-originated action without a user actor.
System{}}}
wire_tagged! {
/// Stable typed reference to the object affected by a committed activity.
pub enum ActivityTarget {
/// Account affected by registration, credential, or email operations.
Account{
    /// Stable identifier of the affected account.
    #[serde(rename="userId")]user_id:UserId,
    /// For `account.email`: the address before the change. Null otherwise.
    #[serde(rename="previousEmail",deserialize_with="nullable")]previous_email:Option<String> [required=true],
},
/// User session affected by the operation.
Session{
    /// Stable identifier of the affected session.
    #[serde(rename="sessionId")]session_id:SessionId
},
/// Network affected by creation or rename.
Network{
    /// Stable identifier of the affected network.
    #[serde(rename="networkId")]network_id:NetworkId
},
/// Membership affected by joining, role change, or removal.
Membership{
    /// Stable identifier of the affected membership.
    #[serde(rename="membershipId")]membership_id:MembershipId
},
/// Join link affected by creation or revocation.
JoinLink{
    /// Stable identifier of the affected join link.
    #[serde(rename="linkId")]link_id:JoinLinkId
},
/// Saved network configuration affected by the operation.
Configuration{
    /// Configuration revision produced by the operation.
    revision:Revision
}}}

macro_rules! actions {
    ($($(#[doc = $doc:literal])* $variant:ident = $wire:literal),* $(,)?) => {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
        #[serde(try_from = "String")]
        /// Completed account or network management action recorded in the same transaction as its effects.
        pub enum ActivityAction { $($(#[doc = $doc])* #[serde(rename = $wire)] $variant),* }
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
                // A string enum has no per-value description slot: list the
                // variants' doc comments in the type description instead.
                let description = [$(concat!("`", $wire, "`: ", $($doc,)* "")),*]
                    .iter()
                    .map(|line| line.trim_end_matches([':', ' ']).trim().replace(":  ", ": "))
                    .collect::<Vec<_>>()
                    .join("\n");
                ObjectBuilder::new().schema_type(Type::String)
                    .description(Some(description))
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

wire_object! {
/// Immutable record of a completed management action, excluding secrets and failed requests.
pub struct ActivityEvent {
    /// Opaque identifier of this activity record.
    pub id:ActivityId,
    /// Server time at which the action completed.
    pub at:DateTime<Utc>,
    /// Verified user or system that performed the action.
    pub actor:ActivityActor,
    /// Completed management operation represented by this record.
    pub action:ActivityAction,
    /// Stable typed reference to the affected object.
    pub target:ActivityTarget,
    /// Configuration revision before the action, or null when no configuration change was involved.
    #[serde(deserialize_with="nullable")]pub before_revision:Option<Revision> [required=true],
    /// Configuration revision after the action, or null when no configuration change was involved.
    #[serde(deserialize_with="nullable")]pub after_revision:Option<Revision> [required=true],
    /// Correlation ID for the logical operation, not an authorization credential.
    pub request_id:RequestId,
} }
wire_object! {
/// Bounded activity page with a fixed initial commit-sequence upper bound.
pub struct ActivityListResponse {
    /// Server-generated correlation ID for this HTTP request.
    pub request_id:RequestId,
    /// Authorized activity records in descending commit order.
    pub items:Vec<ActivityEvent>,
    /// Opaque identity- and scope-bound continuation cursor, or null at the end.
    #[serde(deserialize_with="nullable")]pub next_cursor:Option<String> [required=true],
    /// Time at which the initial pagination window was established.
    pub as_of:DateTime<Utc>
} }
wire_object! {
/// One activity record returned under the caller's current network permissions.
pub struct ActivityResponse {
    /// Server-generated correlation ID for this HTTP request.
    pub request_id:RequestId,
    /// Committed activity belonging to the requested network.
    pub activity:ActivityEvent
} }

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
/// Primary trigger for a saved configuration revision, independent of Node application status.
pub enum ConfigurationChangeCause {
    /// Initial configuration saved when the network was created.
    NetworkCreated,
    /// Configuration saved through a normal edit.
    Edit,
    /// New revision created by restoring a historical configuration.
    Restore,
    /// Configuration changed by Node enrollment.
    Enrollment,
    /// Configuration changed by permanent Node deletion.
    NodeDeleted,
    /// User email references synchronized after an account email change.
    EmailSync,
    /// Configuration changed by membership removal and its Node cleanup.
    MemberRemoved,
    /// Node identity changed through an authorized identity transition.
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
wire_object! {
/// Non-secret summary of one persistently saved configuration version.
pub struct ConfigurationRevisionSummary {
    /// Historical configuration revision represented by this entry.
    pub revision:Revision,
    /// Time at which this revision was persistently committed.
    pub created_at:DateTime<Utc>,
    /// Verified committing actor with its historical email snapshot when available.
    pub created_by:ActivityActor,
    /// Primary trigger that produced this configuration revision.
    pub cause:ConfigurationChangeCause,
    /// Whether this revision equals the current desired revision at this read.
    pub current:bool,
    /// Source revision used for a restore, or null for a non-restore change.
    #[serde(deserialize_with="nullable")]pub rollback_from:Option<Revision> [required=true]
} }
wire_object! {
/// Configuration history page bounded by the maximum revision of the initial query.
pub struct ConfigurationRevisionList {
    /// Server-generated correlation ID for this HTTP request.
    pub request_id:RequestId,
    /// Current desired configuration revision at this page read.
    pub current_revision:Revision,
    /// Version summaries in descending revision order.
    pub items:Vec<ConfigurationRevisionSummary>,
    /// Opaque identity- and network-bound continuation cursor, or null at the end.
    #[serde(deserialize_with="nullable")]pub next_cursor:Option<String> [required=true],
    /// Time at which the initial pagination window was established.
    pub as_of:DateTime<Utc>
} }
