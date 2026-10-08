// Generated from Controller OpenAPI. Do not edit.

#[derive(Clone)]
pub struct AcceptUdpTraversalPath {
    pub r#attempt_id: meshway_protocol::UdpTraversalId,
}

#[derive(Clone)]
pub struct ApplyConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct ApproveDeviceAuthorizationPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct CancelEnrollmentRequestPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct CancelNodeObservationPath {
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}

#[derive(Clone)]
pub struct CancelObservationPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}

#[derive(Clone)]
pub struct CancelUdpTraversalPath {
    pub r#attempt_id: meshway_protocol::UdpTraversalId,
}

#[derive(Clone)]
pub struct ChangeMemberRolePath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
}

#[derive(Clone)]
pub struct CompareConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct CreateEnrollmentKeyPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct CreateJoinLinkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct CreateObservationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct DeleteNodePath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#node_id: meshway_protocol::NodeId,
}

#[derive(Clone)]
pub struct DenyDeviceAuthorizationPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct EvaluateConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetConfigurationProgressPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#revision: meshway_protocol::Revision,
}

#[derive(Clone)]
pub struct GetConfigurationRevisionPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#revision: meshway_protocol::Revision,
}

#[derive(Clone)]
pub struct GetConfigurationSourcePath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct GetConfigurationSourceQuery {
    pub r#revision: Option<meshway_protocol::Revision>,
}

#[derive(Clone)]
pub struct GetDeviceAuthorizationPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct GetExecutionPublicationPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#publication_id: meshway_protocol::PublicationId,
}

#[derive(Clone)]
pub struct GetLeaveImpactPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetMemberRemovalImpactPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
}

#[derive(Clone)]
pub struct GetMemberRoleChangeImpactPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
    pub r#role: meshway_protocol::MembershipRole,
}

#[derive(Clone)]
pub struct GetNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetNetworkActivityPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#activity_id: meshway_protocol::ActivityId,
}

#[derive(Clone)]
pub struct GetNetworkStatePath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetNodePath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#node_id: meshway_protocol::NodeId,
}

#[derive(Clone)]
pub struct GetNodeObservationPath {
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}
#[derive(Clone)]
pub struct GetNodeObservationQuery {
    pub r#generation: meshway_protocol::Version,
    pub r#instance_id: meshway_protocol::InstanceId,
}

#[derive(Clone)]
pub struct GetNodeSettingsPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#node_id: meshway_protocol::NodeId,
}

#[derive(Clone)]
pub struct GetNodeStatePath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#node_id: meshway_protocol::NodeId,
}

#[derive(Clone)]
pub struct GetObservationPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}

#[derive(Clone)]
pub struct GetRuleSetPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#rule_set_key: String,
}

#[derive(Clone)]
pub struct LeaveNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone, Default)]
pub struct ListAccountActivityQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone, Default)]
pub struct ListAccountSessionsQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListConfigurationRevisionsPath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct ListConfigurationRevisionsQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListEnrollmentKeysPath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct ListEnrollmentKeysQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListJoinLinksPath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct ListJoinLinksQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListNetworkActivityPath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct ListNetworkActivityQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListNetworkMembersPath {
    pub r#network_id: meshway_protocol::NetworkId,
}
#[derive(Clone, Default)]
pub struct ListNetworkMembersQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone, Default)]
pub struct ListNetworksQuery {
    pub r#cursor: Option<String>,
    pub r#limit: Option<i32>,
}

#[derive(Clone)]
pub struct ListNodesPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct ListObservationsPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct ListRuleSetsPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct QueryEnrollmentRequestPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct QueryObservationHistoryPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct RedeemEnrollmentRequestPath {
    pub r#enrollment_request_id: meshway_protocol::EnrollmentRequestId,
}

#[derive(Clone)]
pub struct RefreshRuleSetPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#rule_set_key: String,
}

#[derive(Clone)]
pub struct RemoveMemberPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
}

#[derive(Clone)]
pub struct RenameNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct RenewNodeObservationPath {
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}

#[derive(Clone)]
pub struct RenewObservationPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#subscription_id: meshway_protocol::ObservationSubscriptionId,
}

#[derive(Clone)]
pub struct ReportTaskResultPath {
    pub r#task_id: meshway_protocol::ObservationId,
}

#[derive(Clone)]
pub struct RevokeAccountSessionPath {
    pub r#session_id: meshway_protocol::SessionId,
}

#[derive(Clone)]
pub struct RevokeEnrollmentKeyPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#key_id: meshway_protocol::EnrollmentKeyId,
}

#[derive(Clone)]
pub struct RevokeJoinLinkPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#link_id: meshway_protocol::JoinLinkId,
}

#[derive(Clone)]
pub struct RevokeNodeKeyPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#node_id: meshway_protocol::NodeId,
}

#[derive(Clone)]
pub struct ValidateConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct WatchNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

impl crate::Client {
    pub async fn accept_udp_traversal(
        &self,
        path: &AcceptUdpTraversalPath,
        body: &meshway_protocol::UdpTraversalAttemptRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::AcceptUdpTraversalResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("attemptId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#attempt_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/connectivity/udp/attempts/{attemptId}/accept",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn acquire_node_instance(
        &self,
        body: &meshway_protocol::AcquireInstanceRequest,
        token: &str,
    ) -> Result<meshway_protocol::InstanceLease, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/instances",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn apply_configuration(
        &self,
        path: &ApplyConfigurationPath,
        body: &meshway_protocol::ConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationAppliedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn approve_device_authorization(
        &self,
        path: &ApproveDeviceAuthorizationPath,
        body: &meshway_protocol::ApproveEnrollmentRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::DeviceAuthorizationView,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/device-authorizations/{enrollmentRequestId}/approve",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 409 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn cancel_enrollment_request(
        &self,
        path: &CancelEnrollmentRequestPath,
        body: &meshway_protocol::EnrollmentDeviceProof,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/cancel",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, None, Some(body))
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 409 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn cancel_node_demand(
        &self,
        body: &meshway_protocol::CancelDemandRequest,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/demands/cancel",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn cancel_node_observation(
        &self,
        path: &CancelNodeObservationPath,
        body: &meshway_protocol::InstanceRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeObservationCancelled,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("subscriptionId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#subscription_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/observations/{subscriptionId}/cancel",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn cancel_observation(
        &self,
        path: &CancelObservationPath,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("subscriptionId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#subscription_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/observations/{subscriptionId}/cancel",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn cancel_udp_traversal(
        &self,
        path: &CancelUdpTraversalPath,
        body: &meshway_protocol::UdpTraversalAttemptRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::CancelUdpTraversalResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("attemptId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#attempt_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn change_account_email(
        &self,
        body: &meshway_protocol::ChangeEmailRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::AccountResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/account/email",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn change_account_password(
        &self,
        body: &meshway_protocol::ChangePasswordRequest,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/account/password",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 413 | 415 | 422 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn change_member_role(
        &self,
        path: &ChangeMemberRolePath,
        body: &meshway_protocol::ChangeMemberRoleRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::MemberMutationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("membershipId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#membership_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/members/{membershipId}/role",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn compare_configuration(
        &self,
        path: &CompareConfigurationPath,
        body: &meshway_protocol::CompareConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationComparison,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/compare",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_enrollment_key(
        &self,
        path: &CreateEnrollmentKeyPath,
        body: &meshway_protocol::CreateEnrollmentKeyRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::EnrollmentKeyResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/enrollment-keys",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 409 | 422 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_enrollment_request(
        &self,
        body: &meshway_protocol::CreateEnrollmentRequest,
    ) -> Result<
        meshway_protocol::EnrollmentRequestView,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/enrollment-requests",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_join_link(
        &self,
        path: &CreateJoinLinkPath,
        body: &meshway_protocol::CreateJoinLinkRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::JoinLinkCreated,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/join-links",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            201 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_network(
        &self,
        body: &meshway_protocol::CreateNetworkRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NetworkResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/networks",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            201 => response.json(),
            400 | 401 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_node_challenge(
        &self,
        body: &meshway_protocol::SignedNodeChallengeRequest,
    ) -> Result<meshway_protocol::NodeChallenge, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/auth/challenges",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_node_demand(
        &self,
        body: &meshway_protocol::DemandRequest,
        token: &str,
    ) -> Result<meshway_protocol::DemandResponse, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/demands",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_node_observation(
        &self,
        body: &meshway_protocol::CreateNodeObservation,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeObservationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/observations",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            201 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_node_session(
        &self,
        body: &meshway_protocol::NodeSessionRequest,
    ) -> Result<
        meshway_protocol::NodeSessionResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/auth/session",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn create_observation(
        &self,
        path: &CreateObservationPath,
        body: &meshway_protocol::CreateObservation,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationTask,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/observations",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            201 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn delete_node(
        &self,
        path: &DeleteNodePath,
        body: &meshway_protocol::DeleteNodeRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationAppliedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("nodeId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#node_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes/{nodeId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::DELETE, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 409 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn deny_device_authorization(
        &self,
        path: &DenyDeviceAuthorizationPath,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/device-authorizations/{enrollmentRequestId}/deny",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            204 => response.empty(),
            401 | 403 | 409 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn enroll_node_with_key(
        &self,
        body: &meshway_protocol::KeyEnrollmentRequest,
    ) -> Result<
        meshway_protocol::EnrollmentResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/enrollments",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn evaluate_configuration(
        &self,
        path: &EvaluateConfigurationPath,
        body: &meshway_protocol::EvaluateConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::EvaluationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/evaluate",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_account(
        &self,
        token: &str,
    ) -> Result<
        meshway_protocol::AccountResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/api/v1/account",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_auth_options(
        &self,
    ) -> Result<
        meshway_protocol::AuthOptionsResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/api/v1/auth/options",
                None,
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_auth_session(
        &self,
        token: &str,
    ) -> Result<
        meshway_protocol::AuthSessionResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/api/v1/auth/session",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_configuration(
        &self,
        path: &GetConfigurationPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_configuration_progress(
        &self,
        path: &GetConfigurationProgressPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationApplyProgress,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("revision", {
                let value = serde_json::from_slice::<i64>(
                    &serde_json::to_vec(&path.r#revision)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(1..=9007199254740991).contains(&value) {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/revisions/{revision}/progress",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_configuration_revision(
        &self,
        path: &GetConfigurationRevisionPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("revision", {
                let value = serde_json::from_slice::<i64>(
                    &serde_json::to_vec(&path.r#revision)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(1..=9007199254740991).contains(&value) {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/revisions/{revision}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_configuration_schema(
        &self,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationSchemaResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/api/v1/configuration/schema",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_configuration_source(
        &self,
        path: &GetConfigurationSourcePath,
        query: &GetConfigurationSourceQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationSourceResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [(
            "revision",
            query
                .r#revision
                .as_ref()
                .map(|value| {
                    Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                        let value = serde_json::from_slice::<i64>(
                            &serde_json::to_vec(value).map_err(|_| crate::ClientError::Request)?,
                        )
                        .map_err(|_| crate::ClientError::Request)?;
                        if !(1..=9007199254740991).contains(&value) {
                            return Err(crate::ClientError::Request);
                        }
                        value.to_string()
                    })
                })
                .transpose()?,
        )];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/source",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_device_authorization(
        &self,
        path: &GetDeviceAuthorizationPath,
        token: &str,
    ) -> Result<
        meshway_protocol::DeviceAuthorizationView,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/device-authorizations/{enrollmentRequestId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            401 | 404 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_execution_publication(
        &self,
        path: &GetExecutionPublicationPath,
        token: &str,
    ) -> Result<
        meshway_protocol::PublicationProgress,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("publicationId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#publication_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/publications/{publicationId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            401 | 403 | 404 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_leave_impact(
        &self,
        path: &GetLeaveImpactPath,
        token: &str,
    ) -> Result<
        meshway_protocol::RemovalImpactResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/leave-impact",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_member_removal_impact(
        &self,
        path: &GetMemberRemovalImpactPath,
        token: &str,
    ) -> Result<
        meshway_protocol::RemovalImpactResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("membershipId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#membership_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/members/{membershipId}/removal-impact",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_member_role_change_impact(
        &self,
        path: &GetMemberRoleChangeImpactPath,
        token: &str,
    ) -> Result<
        meshway_protocol::RoleChangeImpactResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("membershipId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#membership_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("role", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#role).map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !["owner", "admin", "member"].contains(&value.as_str()) {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/members/{membershipId}/role-change-impact/{role}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_network(
        &self,
        path: &GetNetworkPath,
        token: &str,
    ) -> Result<
        meshway_protocol::NetworkResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url =
            self.parameter_url("/api/v1/networks/{networkId}", &path_values, &query_values)?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_network_activity(
        &self,
        path: &GetNetworkActivityPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ActivityResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("activityId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#activity_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/activity/{activityId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_network_state(
        &self,
        path: &GetNetworkStatePath,
        token: &str,
    ) -> Result<meshway_protocol::StateView, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/state",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_node(
        &self,
        path: &GetNodePath,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeInventoryView,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("nodeId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#node_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes/{nodeId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            401 | 404 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_node_observation(
        &self,
        path: &GetNodeObservationPath,
        query: &GetNodeObservationQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeObservationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("subscriptionId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#subscription_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "generation",
                Some({
                    let value = serde_json::from_slice::<i64>(
                        &serde_json::to_vec(&query.r#generation)
                            .map_err(|_| crate::ClientError::Request)?,
                    )
                    .map_err(|_| crate::ClientError::Request)?;
                    if !(1..=9007199254740991).contains(&value) {
                        return Err(crate::ClientError::Request);
                    }
                    value.to_string()
                }),
            ),
            (
                "instanceId",
                Some({
                    let value = serde_json::from_slice::<String>(
                        &serde_json::to_vec(&query.r#instance_id)
                            .map_err(|_| crate::ClientError::Request)?,
                    )
                    .map_err(|_| crate::ClientError::Request)?;
                    if !(value.len() == 36
                        && value != "00000000-0000-0000-0000-000000000000"
                        && value.bytes().enumerate().all(|(index, byte)| {
                            if matches!(index, 8 | 13 | 18 | 23) {
                                byte == b'-'
                            } else {
                                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                            }
                        }))
                        || value.chars().count() < 36
                        || value.chars().count() > 36
                    {
                        return Err(crate::ClientError::Request);
                    }
                    value.to_string()
                }),
            ),
        ];
        let url = self.parameter_url(
            "/agent-api/v1/observations/{subscriptionId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_node_settings(
        &self,
        path: &GetNodeSettingsPath,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeSettingsResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("nodeId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#node_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes/{nodeId}/settings",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_node_state(
        &self,
        path: &GetNodeStatePath,
        token: &str,
    ) -> Result<meshway_protocol::StateView, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("nodeId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#node_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes/{nodeId}/state",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_node_tasks(
        &self,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationList,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/agent-api/v1/tasks",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_observation(
        &self,
        path: &GetObservationPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationList,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("subscriptionId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#subscription_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/observations/{subscriptionId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_rule_set(
        &self,
        path: &GetRuleSetPath,
        token: &str,
    ) -> Result<meshway_protocol::RuleSetList, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("ruleSetKey", {
                let value = {
                    let input = &path.r#rule_set_key;
                    input.to_owned()
                };
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn get_transport_catalog(
        &self,
        token: &str,
    ) -> Result<
        meshway_protocol::TransportCatalogResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::GET,
                "/api/v1/transport-catalog",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            200 => response.json(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn inspect_join_link(
        &self,
        body: &meshway_protocol::JoinLinkCredentialRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::JoinLinkInspection,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/join-links/inspect",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn inspect_password_reset(
        &self,
        body: &meshway_protocol::PasswordResetInspectRequest,
    ) -> Result<
        meshway_protocol::PasswordResetInspection,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/auth/password-reset/inspect",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn leave_network(
        &self,
        path: &LeaveNetworkPath,
        body: &meshway_protocol::RemoveMemberRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::MemberRemovedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/leave",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_account_activity(
        &self,
        query: &ListAccountActivityQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::ActivityListResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url("/api/v1/account/activity", &path_values, &query_values)?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_account_sessions(
        &self,
        query: &ListAccountSessionsQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::AccountSessionsResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url("/api/v1/account/sessions", &path_values, &query_values)?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_configuration_revisions(
        &self,
        path: &ListConfigurationRevisionsPath,
        query: &ListConfigurationRevisionsQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationRevisionList,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/revisions",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_enrollment_keys(
        &self,
        path: &ListEnrollmentKeysPath,
        query: &ListEnrollmentKeysQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::EnrollmentKeysResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/enrollment-keys",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_join_links(
        &self,
        path: &ListJoinLinksPath,
        query: &ListJoinLinksQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::JoinLinkListResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/join-links",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_network_activity(
        &self,
        path: &ListNetworkActivityPath,
        query: &ListNetworkActivityQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::ActivityListResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/activity",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_network_members(
        &self,
        path: &ListNetworkMembersPath,
        query: &ListNetworkMembersQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::NetworkMembersResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/members",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_networks(
        &self,
        query: &ListNetworksQuery,
        token: &str,
    ) -> Result<
        meshway_protocol::NetworkListResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [];
        let query_values = [
            (
                "cursor",
                query
                    .r#cursor
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if value.chars().count() > 4096 {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
            (
                "limit",
                query
                    .r#limit
                    .as_ref()
                    .map(|value| {
                        Ok::<_, crate::ClientError<meshway_protocol::ErrorResponse>>({
                            let value = {
                                let input = value;
                                input.to_owned()
                            };
                            if !(1..=200).contains(&value) {
                                return Err(crate::ClientError::Request);
                            }
                            value.to_string()
                        })
                    })
                    .transpose()?,
            ),
        ];
        let url = self.parameter_url("/api/v1/networks", &path_values, &query_values)?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_nodes(
        &self,
        path: &ListNodesPath,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeInventoryResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            401 | 404 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_observations(
        &self,
        path: &ListObservationsPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationList,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/observations",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn list_rule_sets(
        &self,
        path: &ListRuleSetsPath,
        token: &str,
    ) -> Result<meshway_protocol::RuleSetList, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/rule-sets",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::GET, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn login(
        &self,
        body: &meshway_protocol::LoginRequest,
    ) -> Result<
        meshway_protocol::SessionIssuedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/auth/login",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn logout(
        &self,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/auth/logout",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            204 => response.empty(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn prepare_udp_traversal(
        &self,
        body: &meshway_protocol::PrepareUdpTraversalRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::PrepareUdpTraversalResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/connectivity/udp/attempts",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_enrollment_request(
        &self,
        path: &QueryEnrollmentRequestPath,
        body: &meshway_protocol::EnrollmentDeviceProof,
    ) -> Result<
        meshway_protocol::EnrollmentPollResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/query",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, None, Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 404 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_node_configuration(
        &self,
        body: &meshway_protocol::NodeConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeConfigurationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/configuration/query",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_node_dns(
        &self,
        body: &meshway_protocol::NodeDnsQueryRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeDnsQueryResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/dns/query",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_observation_history(
        &self,
        path: &QueryObservationHistoryPath,
        body: &meshway_protocol::HistoryQuery,
        token: &str,
    ) -> Result<meshway_protocol::HistoryPage, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/history/query",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_peer_state(
        &self,
        body: &meshway_protocol::StateQuery,
        token: &str,
    ) -> Result<meshway_protocol::StateView, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/state/query",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn query_udp_traversal(
        &self,
        body: &meshway_protocol::QueryUdpTraversalRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::QueryUdpTraversalResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/connectivity/udp/query",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn redeem_enrollment_request(
        &self,
        path: &RedeemEnrollmentRequestPath,
        body: &meshway_protocol::RedeemEnrollmentRequest,
    ) -> Result<
        meshway_protocol::EnrollmentResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("enrollmentRequestId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#enrollment_request_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/redeem",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, None, Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn redeem_join_link(
        &self,
        body: &meshway_protocol::JoinLinkCredentialRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::JoinLinkRedeemed,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/join-links/redeem",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn redeem_password_reset(
        &self,
        body: &meshway_protocol::PasswordResetRedeemRequest,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/auth/password-reset/redeem",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 413 | 415 | 422 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn refresh_node_configuration(
        &self,
        body: &meshway_protocol::NodeConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeConfigurationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/configuration/refresh",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn refresh_rule_set(
        &self,
        path: &RefreshRuleSetPath,
        body: &meshway_protocol::RefreshRuleSetRequest,
        token: &str,
    ) -> Result<meshway_protocol::RuleSetView, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("ruleSetKey", {
                let value = {
                    let input = &path.r#rule_set_key;
                    input.to_owned()
                };
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/refresh",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn register(
        &self,
        body: &meshway_protocol::RegisterRequest,
    ) -> Result<
        meshway_protocol::SessionIssuedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/auth/register",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            201 => response.json(),
            400 | 403 | 409 | 413 | 415 | 422 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn release_node_instance(
        &self,
        body: &meshway_protocol::InstanceRequest,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/instances/release",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn remove_member(
        &self,
        path: &RemoveMemberPath,
        body: &meshway_protocol::RemoveMemberRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::MemberRemovedResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("membershipId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#membership_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/members/{membershipId}/remove",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn rename_network(
        &self,
        path: &RenameNetworkPath,
        body: &meshway_protocol::RenameNetworkRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::NetworkResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/rename",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn renew_node_demand(
        &self,
        body: &meshway_protocol::RenewDemandRequest,
        token: &str,
    ) -> Result<meshway_protocol::DemandResponse, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/demands/renew",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn renew_node_instance(
        &self,
        body: &meshway_protocol::InstanceRequest,
        token: &str,
    ) -> Result<meshway_protocol::InstanceLease, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/instances/renew",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn renew_node_observation(
        &self,
        path: &RenewNodeObservationPath,
        body: &meshway_protocol::RenewNodeObservation,
        token: &str,
    ) -> Result<
        meshway_protocol::NodeObservationResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("subscriptionId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#subscription_id)
                    .map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/observations/{subscriptionId}/renew",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn renew_observation(
        &self,
        path: &RenewObservationPath,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationList,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("subscriptionId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#subscription_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/observations/{subscriptionId}/renew",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn report_node_history(
        &self,
        body: &meshway_protocol::HistoryReport,
        token: &str,
    ) -> Result<
        meshway_protocol::HistoryAccepted,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/history",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn report_node_state(
        &self,
        body: &meshway_protocol::StateReport,
        token: &str,
    ) -> Result<meshway_protocol::StateAccepted, crate::ClientError<meshway_protocol::ErrorResponse>>
    {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/state",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn report_task_result(
        &self,
        path: &ReportTaskResultPath,
        body: &meshway_protocol::ObservationResult,
        token: &str,
    ) -> Result<
        meshway_protocol::ObservationTask,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("taskId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#task_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/agent-api/v1/tasks/{taskId}/results",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 429 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_account_session(
        &self,
        path: &RevokeAccountSessionPath,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [("sessionId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#session_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/account/sessions/{sessionId}/revoke",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 404 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_all_account_sessions(
        &self,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/account/sessions/revoke-all",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            204 => response.empty(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_enrollment_key(
        &self,
        path: &RevokeEnrollmentKeyPath,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("keyId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#key_id).map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/enrollment-keys/{keyId}",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::DELETE, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 403 | 404 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_join_link(
        &self,
        path: &RevokeJoinLinkPath,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("linkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#link_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/join-links/{linkId}/revoke",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), None::<&()>)
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 403 | 404 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_node_key(
        &self,
        path: &RevokeNodeKeyPath,
        body: &meshway_protocol::RevokeNodeKeyRequest,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let path_values = [
            ("networkId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#network_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
            ("nodeId", {
                let value = serde_json::from_slice::<String>(
                    &serde_json::to_vec(&path.r#node_id)
                        .map_err(|_| crate::ClientError::Request)?,
                )
                .map_err(|_| crate::ClientError::Request)?;
                if !(value.len() == 36
                    && value != "00000000-0000-0000-0000-000000000000"
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    }))
                    || value.chars().count() < 36
                    || value.chars().count() > 36
                {
                    return Err(crate::ClientError::Request);
                }
                value.to_string()
            }),
        ];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/nodes/{nodeId}/keys/revoke",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 403 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn revoke_other_account_sessions(
        &self,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/api/v1/account/sessions/revoke-others",
                Some(token),
                None::<&()>,
            )
            .await?;
        match response.status {
            204 => response.empty(),
            401 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn rotate_node_keys(
        &self,
        body: &meshway_protocol::NodeRotationRequest,
    ) -> Result<
        meshway_protocol::NodeKeyMetadata,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/auth/rotate",
                None,
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn select_node_routing_profile(
        &self,
        body: &meshway_protocol::RoutingSelectionRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::RoutingSelectionResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/routing-selection",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn submit_node_configuration_receipt(
        &self,
        body: &meshway_protocol::ReceiptRequest,
        token: &str,
    ) -> Result<(), crate::ClientError<meshway_protocol::ErrorResponse>> {
        let response = self
            .send(
                reqwest::Method::POST,
                "/agent-api/v1/configuration/receipts",
                Some(token),
                Some(body),
            )
            .await?;
        match response.status {
            204 => response.empty(),
            400 | 401 | 409 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn validate_configuration(
        &self,
        path: &ValidateConfigurationPath,
        body: &meshway_protocol::ConfigurationRequest,
        token: &str,
    ) -> Result<
        meshway_protocol::ConfigurationPreviewResponse,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/configuration/validate",
            &path_values,
            &query_values,
        )?;
        let response = self
            .send_url(reqwest::Method::POST, url, Some(token), Some(body))
            .await?;
        match response.status {
            200 => response.json(),
            400 | 401 | 403 | 404 | 409 | 413 | 415 | 422 | 503 => Err(response.problem()),
            _ => Err(response.unexpected()),
        }
    }

    pub async fn watch_network(
        &self,
        path: &WatchNetworkPath,
        token: &str,
        options: &crate::StreamOptions<'_>,
    ) -> Result<
        crate::EventStream<meshway_protocol::UserEvent, meshway_protocol::ErrorResponse>,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let path_values = [("networkId", {
            let value = serde_json::from_slice::<String>(
                &serde_json::to_vec(&path.r#network_id).map_err(|_| crate::ClientError::Request)?,
            )
            .map_err(|_| crate::ClientError::Request)?;
            if !(value.len() == 36
                && value != "00000000-0000-0000-0000-000000000000"
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
                || value.chars().count() < 36
                || value.chars().count() > 36
            {
                return Err(crate::ClientError::Request);
            }
            value.to_string()
        })];
        let query_values = [];
        let url = self.parameter_url(
            "/api/v1/networks/{networkId}/events",
            &path_values,
            &query_values,
        )?;
        self.stream(
            url,
            Some(token),
            options,
            crate::StreamResponse::json(200, &[400, 401, 403, 404, 409, 429, 503]),
        )
        .await
    }

    pub async fn watch_node(
        &self,
        token: &str,
        options: &crate::StreamOptions<'_>,
    ) -> Result<
        crate::EventStream<meshway_protocol::NodeEvent, meshway_protocol::ErrorResponse>,
        crate::ClientError<meshway_protocol::ErrorResponse>,
    > {
        let url = self.parameter_url("/agent-api/v1/events", &[], &[])?;
        self.stream(
            url,
            Some(token),
            options,
            crate::StreamResponse::json(200, &[400, 401, 403, 404, 409, 429, 503]),
        )
        .await
    }
}
