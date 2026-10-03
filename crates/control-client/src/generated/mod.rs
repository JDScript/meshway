// Generated from Controller OpenAPI. Do not edit.

#[derive(Clone)]
pub struct ApplyConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
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
pub struct CreateJoinLinkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
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
pub struct GetLeaveImpactPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct GetMemberRemovalImpactPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
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
pub struct RemoveMemberPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#membership_id: meshway_protocol::MembershipId,
}

#[derive(Clone)]
pub struct RenameNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

#[derive(Clone)]
pub struct RevokeAccountSessionPath {
    pub r#session_id: meshway_protocol::SessionId,
}

#[derive(Clone)]
pub struct RevokeJoinLinkPath {
    pub r#network_id: meshway_protocol::NetworkId,
    pub r#link_id: meshway_protocol::JoinLinkId,
}

#[derive(Clone)]
pub struct ValidateConfigurationPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

impl crate::Client {
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
            400 | 401 | 403 | 404 | 503 => Err(response.problem()),
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
}
