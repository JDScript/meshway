// Generated from Controller OpenAPI. Do not edit.

impl crate::Client {
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
}
