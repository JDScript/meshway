// Generated from Controller OpenAPI. Do not edit.

#[derive(Clone)]
pub struct GetNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
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
pub struct RenameNetworkPath {
    pub r#network_id: meshway_protocol::NetworkId,
}

impl crate::Client {
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
}
