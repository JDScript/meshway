//! Human account API. Web uses these server-to-server and keeps userSession in
//! Secure/HttpOnly/SameSite cookies; Controller accepts no browser session cookie.
use crate::{
    AppState, Error,
    accounts::{Email, Password, PasswordWorker},
    bearer, json,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{ErrorResponse, accounts::*};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
use utoipa_axum::{router::OpenApiRouter, routes};

pub(crate) struct Runtime {
    worker: PasswordWorker,
    budget: Mutex<RateBudget>,
}
struct RateBudget {
    start: Instant,
    total: u32,
    keys: HashMap<[u8; 32], u32>,
}
impl Runtime {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self {
            worker: PasswordWorker::new_blocking(2)?,
            budget: Mutex::new(RateBudget {
                start: Instant::now(),
                total: 0,
                keys: HashMap::new(),
            }),
        })
    }
    fn reserve(
        &self,
        source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
        account: &str,
    ) -> Result<(), Error> {
        let mut budget = self.budget.lock().map_err(|_| Error::Worker)?;
        if budget.start.elapsed() >= Duration::from_secs(60) {
            budget.start = Instant::now();
            budget.total = 0;
            budget.keys.clear();
        }
        let source = source.map_or_else(|| "unknown-source".into(), |s| s.0.0.ip().to_string());
        let source: [u8; 32] = Sha256::digest(format!("source:{source}")).into();
        let account: [u8; 32] = Sha256::digest(format!("account:{account}")).into();
        if budget.total >= 120
            || budget.keys.len() >= 4096
            || budget.keys.get(&source).copied().unwrap_or(0) >= 30
            || budget.keys.get(&account).copied().unwrap_or(0) >= 10
        {
            return Err(crate::accounts::AccountError::Busy.into());
        }
        budget.total += 1;
        *budget.keys.entry(source).or_default() += 1;
        *budget.keys.entry(account).or_default() += 1;
        Ok(())
    }
}

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(session))
        .routes(routes!(logout))
        .routes(routes!(register))
        .routes(routes!(accept))
        .routes(routes!(claim_owner))
        .routes(routes!(members))
        .routes(routes!(save_member))
        .routes(routes!(leave))
        .routes(routes!(invitations))
        .routes(routes!(create_invitation))
        .routes(routes!(invitation))
        .routes(routes!(revoke_invitation))
        .routes(routes!(revoke_sessions))
        .routes(routes!(change_password))
        .routes(routes!(resets))
        .routes(routes!(issue_reset))
        .routes(routes!(reset))
        .routes(routes!(revoke_reset))
        .routes(routes!(redeem_reset))
        .routes(routes!(ownership))
        .routes(routes!(claim_ownership))
        .layer(axum::extract::DefaultBodyLimit::max(8192))
}

fn receipt() -> Json<AccountReceipt> {
    Json(AccountReceipt { saved: true })
}

#[utoipa::path(post,path="/api/v1/auth/login",operation_id="loginAccount",request_body=LoginRequest,
 responses((status=200,description="Private user session, issued after commit",body=LoginResponse),(status=401,description="Invalid credential",body=ErrorResponse),(status=429,description="Authentication budget exhausted",body=ErrorResponse)))]
async fn login(
    State(state): State<AppState>,
    source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Json<LoginResponse>, Error> {
    let request = json(body)?;
    let email = Email::parse(&request.email)?;
    state.accounts.reserve(source, email.as_str())?;
    let snapshot = state.access(move |s| s.account_credential(&email)).await?;
    let proof = state
        .accounts
        .worker
        .verify(snapshot, Password::new(request.password)?)
        .await?;
    state
        .access(move |s| s.account_login(proof).map(Json))
        .await
}

#[utoipa::path(get,path="/api/v1/auth/session",operation_id="getAccountSession",security(("userSession"=[])),responses((status=200,description="Current user and live network memberships",body=AccountSession),(status=401,description="Session invalid",body=ErrorResponse)))]
async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountSession>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |s| s.account_session(&token).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/auth/logout",operation_id="logoutAccount",security(("userSession"=[])),responses((status=200,description="Current session revoked",body=AccountReceipt),(status=401,description="Session invalid",body=ErrorResponse)))]
async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountReceipt>, Error> {
    let token = bearer(&headers)?;
    state.access(move |s| s.account_logout(&token)).await?;
    Ok(receipt())
}

#[utoipa::path(post,path="/api/v1/auth/invitations/register",operation_id="registerInvitedAccount",request_body=RegisterInvitationRequest,responses((status=200,description="Account and membership saved; login separately",body=AccountMembership),(status=401,description="Invitation invalid",body=ErrorResponse),(status=429,description="Authentication budget exhausted",body=ErrorResponse)))]
async fn register(
    State(state): State<AppState>,
    source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<RegisterInvitationRequest>, JsonRejection>,
) -> Result<Json<AccountMembership>, Error> {
    let request = json(body)?;
    let email = Email::parse(&request.email)?;
    state.accounts.reserve(source, email.as_str())?;
    let intent = state
        .access(move |s| s.account_registration(&request.invitation_token, &email))
        .await?;
    let hash = state
        .accounts
        .worker
        .hash(Password::new(request.password)?)
        .await?;
    state
        .access(move |s| s.account_register(intent, hash).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/auth/invitations/accept",operation_id="acceptAccountInvitation",security(("userSession"=[])),request_body=AcceptInvitationRequest,responses((status=200,description="Membership saved without changing existing password",body=AccountMembership),(status=401,description="Session or invitation invalid",body=ErrorResponse)))]
async fn accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AcceptInvitationRequest>, JsonRejection>,
) -> Result<Json<AccountMembership>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |s| {
            s.account_accept(&token, &request.invitation_token)
                .map(Json)
        })
        .await
}

#[utoipa::path(post,path="/api/v1/networks/{network_id}/owner-claim",operation_id="claimFirstOwner",security(("adminBearer"=[])),params(("network_id"=String,Path)),request_body=ClaimOwnerRequest,responses((status=200,description="Explicit first Owner claim",body=AccountMembership),(status=401,description="Bootstrap or own session required",body=ErrorResponse),(status=409,description="Owner or account already exists",body=ErrorResponse)))]
async fn claim_owner(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<ClaimOwnerRequest>, JsonRejection>,
) -> Result<Json<AccountMembership>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    let bootstrap = token.clone();
    state.access(move |s| s.authorize_admin(&bootstrap)).await?;
    match (
        request.kind,
        request.email,
        request.password,
        request.session_token,
    ) {
        (ClaimOwnerKind::NewAccount, Some(email), Some(password), None) => {
            let email = Email::parse(&email)?;
            state.accounts.reserve(source, email.as_str())?;
            let hash = state.accounts.worker.hash(Password::new(password)?).await?;
            state
                .access(move |s| s.account_claim_new(&token, &id, email, hash).map(Json))
                .await
        }
        (ClaimOwnerKind::ExistingAccount, None, None, Some(session_token)) => {
            state
                .access(move |s| {
                    s.account_claim_existing(&token, &id, &session_token)
                        .map(Json)
                })
                .await
        }
        _ => Err(Error::Invalid("invalid owner claim combination")),
    }
}

#[utoipa::path(get,path="/api/v1/networks/{network_id}/members",operation_id="listAccountMembers",security(("managementBearer"=[])),params(("network_id"=String,Path)),responses((status=200,description="Live memberships in this network",body=AccountMembers),(status=403,description="Network membership required",body=ErrorResponse)))]
async fn members(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AccountMembers>, Error> {
    let t = bearer(&headers)?;
    state
        .access(move |s| s.account_members(&t, &id).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/networks/{network_id}/members/save",operation_id="saveAccountMember",security(("managementBearer"=[])),params(("network_id"=String,Path)),request_body=SaveMembershipRequest,responses((status=200,description="Role saved or member and devices revoked atomically",body=AccountReceipt),(status=403,description="Role ceiling exceeded",body=ErrorResponse),(status=409,description="Last Owner protected",body=ErrorResponse)))]
async fn save_member(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SaveMembershipRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_save_member(&t, &id, r))
        .await?;
    Ok(receipt())
}
#[utoipa::path(post,path="/api/v1/networks/{network_id}/members/leave",operation_id="leaveAccountNetwork",security(("userSession"=[])),params(("network_id"=String,Path)),responses((status=200,description="Own membership and devices revoked",body=AccountReceipt),(status=409,description="Last Owner protected",body=ErrorResponse)))]
async fn leave(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    state.access(move |s| s.account_leave(&t, &id)).await?;
    Ok(receipt())
}

#[utoipa::path(get,path="/api/v1/networks/{network_id}/invitations",operation_id="listAccountInvitations",security(("managementBearer"=[])),params(("network_id"=String,Path)),responses((status=200,description="Public invitation metadata only",body=AccountInvitations),(status=403,description="Network manager required",body=ErrorResponse)))]
async fn invitations(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AccountInvitations>, Error> {
    let t = bearer(&headers)?;
    state
        .access(move |s| s.account_invitations(&t, &id).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/networks/{network_id}/invitations",operation_id="createAccountInvitation",security(("managementBearer"=[])),params(("network_id"=String,Path)),request_body=CreateInvitationRequest,responses((status=200,description="Secret returned once after commit",body=CreateInvitationResponse),(status=403,description="Grant not permitted",body=ErrorResponse)))]
async fn create_invitation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<CreateInvitationRequest>, JsonRejection>,
) -> Result<Json<CreateInvitationResponse>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_invite(&t, &id, r).map(Json))
        .await
}
#[utoipa::path(get,path="/api/v1/networks/{network_id}/invitations/{invitation_id}",operation_id="getAccountInvitation",security(("managementBearer"=[])),params(("network_id"=String,Path),("invitation_id"=String,Path)),responses((status=200,description="Public metadata; secret cannot be retrieved",body=AccountInvitation),(status=403,description="Resource not permitted",body=ErrorResponse)))]
async fn invitation(
    State(state): State<AppState>,
    Path((id, resource)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<AccountInvitation>, Error> {
    let t = bearer(&headers)?;
    state
        .access(move |s| s.account_invitation(&t, &id, &resource).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/networks/{network_id}/invitations/revoke",operation_id="revokeAccountInvitation",security(("managementBearer"=[])),params(("network_id"=String,Path)),request_body=RevokeInvitationRequest,responses((status=200,description="Public resource ID revoked",body=AccountReceipt),(status=403,description="Resource not permitted",body=ErrorResponse)))]
async fn revoke_invitation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<RevokeInvitationRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_revoke_invite(&t, &id, &r.invitation_id))
        .await?;
    Ok(receipt())
}

#[utoipa::path(post,path="/api/v1/auth/sessions/revoke",operation_id="revokeAccountSessions",security(("managementBearer"=[])),request_body=RevokeAccountSessionsRequest,responses((status=200,description="Own or bootstrap-targeted sessions revoked",body=AccountReceipt),(status=403,description="Cannot revoke another user",body=ErrorResponse)))]
async fn revoke_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RevokeAccountSessionsRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_revoke_sessions(&t, &r.user_id))
        .await?;
    Ok(receipt())
}
#[utoipa::path(post,path="/api/v1/me/password",operation_id="changeAccountPassword",security(("userSession"=[])),request_body=ChangePasswordRequest,responses((status=200,description="Password replaced and all old sessions revoked",body=AccountReceipt),(status=401,description="Session or password invalid",body=ErrorResponse),(status=429,description="Authentication budget exhausted",body=ErrorResponse)))]
async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    let t = token.clone();
    let session = state.access(move |s| s.account_session(&t)).await?;
    state.accounts.reserve(source, &session.user.email)?;
    let email = Email::parse(&session.user.email)?;
    let snapshot = state.access(move |s| s.account_credential(&email)).await?;
    let proof = state
        .accounts
        .worker
        .verify(snapshot, Password::new(request.current_password)?)
        .await?;
    let hash = state
        .accounts
        .worker
        .hash(Password::new(request.new_password)?)
        .await?;
    state
        .access(move |s| s.account_change_password(&token, proof, hash))
        .await?;
    Ok(receipt())
}

#[utoipa::path(get,path="/api/v1/auth/password-resets",operation_id="listAccountPasswordResets",security(("adminBearer"=[])),responses((status=200,description="Global bootstrap-only reset inventory",body=AccountPasswordResets),(status=403,description="Instance bootstrap required",body=ErrorResponse)))]
async fn resets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountPasswordResets>, Error> {
    let t = bearer(&headers)?;
    state.access(move |s| s.account_resets(&t).map(Json)).await
}
#[utoipa::path(post,path="/api/v1/auth/password-resets",operation_id="createAccountPasswordReset",security(("adminBearer"=[])),request_body=CreatePasswordResetRequest,responses((status=200,description="Reset secret returned once after commit",body=CreatePasswordResetResponse),(status=403,description="Instance bootstrap required",body=ErrorResponse)))]
async fn issue_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreatePasswordResetRequest>, JsonRejection>,
) -> Result<Json<CreatePasswordResetResponse>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_issue_reset(&t, r).map(Json))
        .await
}
#[utoipa::path(get,path="/api/v1/auth/password-resets/{reset_id}",operation_id="getAccountPasswordReset",security(("adminBearer"=[])),params(("reset_id"=String,Path)),responses((status=200,description="Public reset metadata",body=AccountPasswordReset),(status=403,description="Instance bootstrap required",body=ErrorResponse)))]
async fn reset(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AccountPasswordReset>, Error> {
    let t = bearer(&headers)?;
    state
        .access(move |s| s.account_reset(&t, &id).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/auth/password-resets/revoke",operation_id="revokeAccountPasswordReset",security(("adminBearer"=[])),request_body=RevokePasswordResetRequest,responses((status=200,description="Reset revoked by public ID",body=AccountReceipt),(status=403,description="Instance bootstrap required",body=ErrorResponse)))]
async fn revoke_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RevokePasswordResetRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.account_revoke_reset(&t, &r.reset_id))
        .await?;
    Ok(receipt())
}
#[utoipa::path(post,path="/api/v1/auth/password-resets/redeem",operation_id="redeemAccountPasswordReset",request_body=RedeemPasswordResetRequest,responses((status=200,description="Password replaced; reset consumed and sessions revoked",body=AccountReceipt),(status=401,description="Reset invalid",body=ErrorResponse),(status=429,description="Authentication budget exhausted",body=ErrorResponse)))]
async fn redeem_reset(
    State(state): State<AppState>,
    source: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<RedeemPasswordResetRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let request = json(body)?;
    state.accounts.reserve(source, &request.reset_token)?;
    let intent = state
        .access(move |s| s.account_reset_intent(&request.reset_token))
        .await?;
    state.accounts.reserve(source, intent.account_key())?;
    let hash = state
        .accounts
        .worker
        .hash(Password::new(request.new_password)?)
        .await?;
    state
        .access(move |s| s.account_redeem_reset(intent, hash))
        .await?;
    Ok(receipt())
}

#[utoipa::path(get,path="/api/v1/networks/{network_id}/node-ownership",operation_id="listNodeOwnership",security(("managementBearer"=[])),params(("network_id"=String,Path)),responses((status=200,description="Real ownership; null is legacy-unassigned",body=NodeOwnershipList),(status=403,description="Network membership required",body=ErrorResponse)))]
async fn ownership(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<NodeOwnershipList>, Error> {
    let t = bearer(&headers)?;
    state
        .access(move |s| s.node_ownership(&t, &id).map(Json))
        .await
}
#[utoipa::path(post,path="/api/v1/networks/{network_id}/node-ownership/claim",operation_id="claimNodeOwnership",security(("userSession"=[])),params(("network_id"=String,Path)),request_body=ClaimNodeOwnershipRequest,responses((status=200,description="Network Owner explicitly assigned legacy device",body=AccountReceipt),(status=403,description="Owner or target membership invalid",body=ErrorResponse),(status=409,description="Device already assigned",body=ErrorResponse)))]
async fn claim_ownership(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<ClaimNodeOwnershipRequest>, JsonRejection>,
) -> Result<Json<AccountReceipt>, Error> {
    let t = bearer(&headers)?;
    let r = json(body)?;
    state
        .access(move |s| s.claim_node_ownership(&t, &id, r))
        .await?;
    Ok(receipt())
}
