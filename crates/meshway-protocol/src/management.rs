use crate::{UserView, WireSecret, object::wire_object};
use chrono::{DateTime, Utc};
use meshway_model::{
    JoinLinkId, MembershipId, MembershipRole, NetworkId, NodeId, RequestId, Revision, SessionId,
    UserId, Version,
};

use meshway_serde::required_nullable as nullable;

wire_object! {
    /// The authenticated user's account metadata.
    pub struct AccountResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Current account identifier and canonical email address.
        pub user: UserView
    }
}
wire_object! {
    /// Password replacement authorized by the account's current password.
    pub struct ChangePasswordRequest {
        /// Current account password, without trimming or rewriting.
        pub current_password: WireSecret,
        /// Replacement password satisfying the advertised password rules.
        pub password: WireSecret
    }
}
wire_object! {
    /// Login email replacement authorized by the account password.
    pub struct ChangeEmailRequest {
        /// Current account password used to authorize the change.
        pub password: WireSecret,
        /// New login address, normalized using the registration email rules.
        pub email: String
    }
}
wire_object! {
    /// Secret-bearing request to inspect a password reset without consuming it.
    pub struct PasswordResetInspectRequest {
        /// One-time password reset credential issued by an authorized host operator.
        pub token: WireSecret
    }
}
wire_object! {
    /// One-time password reset redemption that invalidates old user sessions.
    pub struct PasswordResetRedeemRequest {
        /// Unused, unexpired password reset credential for the account's current credential version.
        pub token: WireSecret,
        /// Replacement password satisfying the advertised password rules.
        pub password: WireSecret
    }
}
wire_object! {
    /// Validity information for an inspected password reset credential.
    pub struct PasswordResetInspection {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Time at which the reset credential becomes invalid.
        pub expires_at: DateTime<Utc>
    }
}
wire_object! {
/// Non-secret metadata for one of the authenticated user's active sessions.
pub struct AccountSessionView {
    /// Stable identifier used to revoke this session.
    pub id: SessionId,
    /// Time at which this session was issued.
    pub created_at: DateTime<Utc>,
    /// Absolute session expiry, not extended by session use.
    pub expires_at: DateTime<Utc>,
    /// Last recorded authenticated use, with writes coalesced over 30 seconds; null before first use.
    #[serde(deserialize_with="nullable")] pub last_used_at: Option<DateTime<Utc>> [required=true],
    /// Coarse self-reported client label, or null when the User-Agent is absent or unrecognized.
    #[serde(deserialize_with="nullable")] pub client_label: Option<String> [required=true],
    /// Whether this session authenticated the current request.
    pub current: bool,
    /// The delegation client this session was issued to, or null for a session the user signed in with.
    #[serde(deserialize_with="nullable")] pub delegation_client: Option<String> [required=true],
} }
wire_object! {
/// A bounded page of the authenticated user's active sessions.
pub struct AccountSessionsResponse {
    /// Server-generated correlation ID for this HTTP request.
    pub request_id: RequestId,
    /// Active sessions in descending creation order within the initial query bound.
    pub items: Vec<AccountSessionView>,
    /// Opaque continuation cursor, or null when no further page is available.
    #[serde(deserialize_with="nullable")] pub next_cursor: Option<String> [required=true],
    /// Time at which the initial pagination window was established.
    pub as_of: DateTime<Utc>,
} }

wire_object! {
    /// Membership role change guarded by the previously read member version.
    pub struct ChangeMemberRoleRequest {
        /// Member version that must still match when the role change is committed.
        pub expected_version: Version,
        /// Requested role, subject to the caller's assignable roles and last-Owner protection.
        pub role: MembershipRole
    }
}
wire_object! {
    /// Member removal or network departure guarded by membership and configuration versions.
    pub struct RemoveMemberRequest {
        /// Membership version read when reviewing the removal.
        pub expected_version:Version,
        /// Configuration revision read when reviewing the affected User-identity nodes.
        pub expected_revision:Revision
    }
}
wire_object! {
    /// Current User-identity nodes that would be permanently deleted by membership removal.
    pub struct RemovalImpactResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id:RequestId,
        /// Membership whose removal is being previewed.
        pub membership_id:MembershipId,
        /// Current membership version to use as the removal concurrency guard.
        pub version:Version,
        /// Configuration revision used to compute the removal impact.
        pub revision:Revision,
        /// Complete set of nodes currently bound to this User membership, excluding Tag nodes.
        pub node_ids:Vec<NodeId>,
        /// Number of nodes in the removal set.
        pub node_count:u32
    }
}
// What changing a member to `role` would do right now, besides the role
// itself: the live join links the member could no longer issue are revoked.
// `join_links` lists the first 100 by newest; `join_link_count` is the total.
wire_object! {
    /// Live join links that would be revoked by the proposed membership role change.
    pub struct RoleChangeImpactResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id:RequestId,
        /// Membership whose role change is being previewed.
        pub membership_id:MembershipId,
        /// Current membership version to use as the role-change concurrency guard.
        pub version:Version,
        /// Proposed role used to calculate the impact.
        pub role:MembershipRole,
        /// At most 100 affected unused, unexpired join links, newest first.
        pub join_links:Vec<JoinLinkView>,
        /// Total number of affected links, including those beyond the returned list.
        pub join_link_count:u32
    }
}
wire_object! {
    /// Committed membership removal result.
    pub struct MemberRemovedResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id:RequestId,
        /// Resulting saved configuration revision after membership cleanup.
        pub revision:Revision
    }
}
wire_object! {
    /// Updated membership and current caller-specific action hints.
    pub struct MemberMutationResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Membership metadata after the mutation.
        pub member: crate::MemberSummary
    }
}
wire_object! {
    /// Request to issue a one-time network join link with a fixed role and lifetime.
    pub struct CreateJoinLinkRequest {
        /// Caller-chosen idempotency key bound to the caller, network, and creation input.
        pub request_id: RequestId,
        /// Role granted on redemption, restricted to roles the issuer may currently grant.
        pub role: MembershipRole,
        /// Link lifetime in seconds, from 60 through 604800.
        pub expires_in_seconds: u32 [minimum=60,maximum=604800]
    }
}
wire_object! {
    /// Secret-bearing join link inspection or redemption input.
    pub struct JoinLinkCredentialRequest {
        /// One-time join credential supplied in the request body rather than the URL.
        pub token: WireSecret
    }
}
// A user named by a link. The email is the account's current address and
// stays readable after the user leaves the network.
wire_object! { #[derive(Debug,Clone,PartialEq,Eq)]
/// A join link's issuer or consumer, readable even after leaving the network.
pub struct LinkUser {
    /// Stable identifier of the named account.
    pub user_id: UserId,
    /// Account's current email address, including after membership removal.
    pub email: String
} }
wire_object! { #[derive(Debug,Clone)]
/// Non-secret join link metadata with current revocation eligibility.
pub struct JoinLinkView {
    /// Stable identifier of this join link.
    pub id: JoinLinkId,
    /// Network to which redemption grants membership.
    pub network_id: NetworkId,
    /// Fixed network role granted by this link.
    pub role: MembershipRole,
    /// Time at which the link was issued.
    pub created_at: DateTime<Utc>,
    /// Time at which an unused link becomes invalid.
    pub expires_at: DateTime<Utc>,
    /// Account that issued the link under its then-current membership.
    pub issued_by: LinkUser,
    /// Time of permanent revocation, or null if the link has not been revoked.
    #[serde(deserialize_with="nullable")] pub revoked_at: Option<DateTime<Utc>> [required=true],
    /// Time of successful redemption, or null while the link remains unconsumed.
    #[serde(deserialize_with="nullable")] pub consumed_at: Option<DateTime<Utc>> [required=true],
    /// Account that redeemed the link, or null before successful redemption.
    #[serde(deserialize_with="nullable")] pub consumed_by: Option<LinkUser> [required=true],
    /// Current caller-specific operation hints, rechecked when a mutation is committed.
    pub actions: JoinLinkActions,
} }
wire_object! { #[derive(Debug,Clone)]
/// Current operation hints for a join link, not an authorization credential.
pub struct JoinLinkActions {
    /// Whether the caller may currently revoke this link.
    pub revoke: bool
} }
wire_object! {
    /// Non-secret response for a single join link.
    pub struct JoinLinkResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Join link metadata without its original credential.
        pub link: JoinLinkView
    }
}
wire_object! {
    /// Validated join link preview that does not consume the credential.
    pub struct JoinLinkInspection {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Inspected link's non-secret metadata.
        pub link: JoinLinkView,
        /// Current name of the network the link grants access to.
        pub network_name: String,
        /// Whether the inspecting user already has an active membership in the network.
        pub already_member: bool
    }
}
wire_object! {
/// A bounded page of join link metadata for an authorized network manager.
pub struct JoinLinkListResponse {
    /// Server-generated correlation ID for this HTTP request.
    pub request_id:RequestId,
    /// Join links in descending creation order within the initial query bound.
    pub items:Vec<JoinLinkView>,
    /// Opaque continuation cursor, or null when no further page is available.
    #[serde(deserialize_with="nullable")] pub next_cursor:Option<String> [required=true],
    /// Time at which the initial pagination window was established.
    pub as_of:DateTime<Utc>,
    /// Total links in the authorized network within the initial creation bound.
    pub total:meshway_model::Count,
} }
wire_object! {
    /// Membership created by redemption or recovered by a valid retry from the same consumer.
    pub struct JoinLinkRedeemed {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id:RequestId,
        /// Network joined by redeeming the link.
        pub network_id:NetworkId,
        /// Membership created by this link, which must remain active for retry recovery.
        pub membership_id:MembershipId
    }
}

// Separate branches make unavailable delivery impossible to mistake for a
// recoverable original. Lists and ordinary reads never carry this type.
crate::object::wire_tagged! {tag="secretDelivery";
/// Join link creation result distinguishing first secret delivery from metadata-only retries.
pub enum JoinLinkCreated {
    /// First successful creation response carrying the one-time join credential.
    Available {
        /// Server-generated correlation ID for this HTTP request.
        #[serde(rename = "requestId")]
        request_id: RequestId,
        /// Created join link's non-secret metadata.
        link: JoinLinkView,
        /// Original join credential, delivered only in the first successful creation response.
        secret: WireSecret,
    },
    /// Idempotent creation retry whose original credential cannot be recovered.
    Unavailable {
        /// Server-generated correlation ID for this HTTP request.
        #[serde(rename = "requestId")]
        request_id: RequestId,
        /// Metadata for the previously created join link.
        link: JoinLinkView,
    },
} }
