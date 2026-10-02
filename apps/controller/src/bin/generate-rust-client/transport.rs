//! Checked adapter: injectable transport without duplicating API operations,
//! auth, bounded response decoding or status-first error semantics.

pub(super) fn adapt(
    content: &mut String,
    operations: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    fn replace(
        content: &mut String,
        from: &str,
        to: &str,
        count: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if content.matches(from).count() != count {
            return Err(format!("upstream transport template changed: {from}").into());
        }
        *content = content.replace(from, to);
        Ok(())
    }
    replace(
        content,
        "pub enum HttpError {",
        "pub enum HttpError {\n    /// Failure from an injected internal transport.\n    #[error(\"Injected transport error: {0}\")]\n    Io(#[from] std::io::Error),",
        1,
    )?;
    replace(
        content,
        "matches!(self, Self::Network(_) | Self::Middleware(_) | Self::Timeout)",
        "matches!(self, Self::Network(_) | Self::Middleware(_) | Self::Timeout) || matches!(self, Self::Io(error) if !matches!(error.kind(), std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied))",
        1,
    )?;
    replace(
        content,
        "    http_client: ClientWithMiddleware,",
        "    http_client: ClientWithMiddleware,\n    transport: Option<std::sync::Arc<dyn HttpTransport>>,",
        1,
    )?;
    replace(
        content,
        "            http_client,",
        "            http_client,\n            transport: None,",
        1,
    )?;
    replace(
        content,
        "#[derive(Clone)]\npub struct HttpClient",
        &format!("{TRANSPORT}\n#[derive(Clone)]\npub struct HttpClient"),
        1,
    )?;
    // Only the constructor impl gets the injection method.
    replace(
        content,
        "    /// Set the base URL for all requests",
        INJECTION,
        1,
    )?;
    replace(
        content,
        "let response = req.send().await?;",
        "let response = if let Some(transport) = &self.transport { transport.execute(req.build()?).await? } else { req.send().await? };",
        operations,
    )?;
    Ok(())
}

// Added once before the first impl by the adapter below, not per operation.
const TRANSPORT: &str = r#"/// Transport injection for internal socket protection. Implementations must
/// enforce connection, TLS, proxy, timeout and retirement semantics; generated
/// code never falls back to reqwest after an injected transport fails.
pub trait HttpTransport: Send + Sync {
    fn execute(&self, request: reqwest::Request) -> std::pin::Pin<Box<dyn std::future::Future<Output = HttpResult<reqwest::Response>> + Send + '_>>;
}"#;

const INJECTION: &str = r#"    /// Inject a transport. All generated operations still build their own
    /// authoritative URL, authentication, headers and JSON body, and decode
    /// responses through the same bounded, status-first implementation.
    pub fn with_transport(mut self, transport: std::sync::Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }
    /// Set the base URL for all requests"#;
