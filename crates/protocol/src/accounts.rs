//! Local human accounts. These credentials are never node/enrollment credentials.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Owner,
    Admin,
    Member,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[schema(min_length = 3, max_length = 254)]
    pub email: String,
    #[schema(min_length = 12, max_length = 1024)]
    pub password: String,
}

/// Returned only after durable session commit; the Web server keeps it private.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginResponse {
    pub session_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountUser {
    pub user_id: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountMembership {
    pub membership_id: String,
    pub network_id: String,
    pub user_id: String,
    pub role: AccountRole,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountSession {
    pub user: AccountUser,
    pub memberships: Vec<AccountMembership>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountMembers {
    pub members: Vec<AccountMember>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountMember {
    pub membership: AccountMembership,
    pub user: AccountUser,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountReceipt {
    pub saved: bool,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitationRequest {
    pub email: String,
    pub role: AccountRole,
    pub expires_in_seconds: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountInvitation {
    pub invitation_id: String,
    pub network_id: String,
    pub email: String,
    pub role: AccountRole,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub issuer_user_id: Option<String>,
    pub expires_at: i64,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub revoked_at: Option<i64>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub consumed_by: Option<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub consumed_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountInvitations {
    pub invitations: Vec<AccountInvitation>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitationResponse {
    pub invitation: AccountInvitation,
    pub invitation_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterInvitationRequest {
    pub invitation_token: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptInvitationRequest {
    pub invitation_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeInvitationRequest {
    pub invitation_id: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMembershipRequest {
    pub user_id: String,
    pub role: AccountMembershipChange,
}

#[derive(Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountMembershipChange {
    Owner,
    Admin,
    Member,
    Remove,
}

/// Bootstrap proof comes from adminBearer; existing accounts must prove their own session.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimOwnerRequest {
    pub kind: ClaimOwnerKind,
    pub email: Option<String>,
    pub password: Option<String>,
    pub session_token: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimOwnerKind {
    NewAccount,
    ExistingAccount,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeAccountSessionsRequest {
    pub user_id: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePasswordResetRequest {
    pub user_id: String,
    pub expires_in_seconds: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountPasswordReset {
    pub reset_id: String,
    pub user_id: String,
    pub expires_at: i64,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub revoked_at: Option<i64>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub consumed_at: Option<i64>,
    pub credential_current: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountPasswordResets {
    pub resets: Vec<AccountPasswordReset>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePasswordResetResponse {
    pub reset: AccountPasswordReset,
    pub reset_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokePasswordResetRequest {
    pub reset_id: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RedeemPasswordResetRequest {
    pub reset_token: String,
    pub new_password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimNodeOwnershipRequest {
    pub node_id: String,
    pub membership_id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeOwnership {
    pub node_id: String,
    pub network_id: String,
    /// Null means legacy-unassigned; tags remain ordinary node attributes.
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub membership_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeOwnershipList {
    pub nodes: Vec<NodeOwnership>,
}
