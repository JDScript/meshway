//! Uniform opaque Debug for generated HTTP DTOs, including every secret-bearing
//! nested container. Independent of HTTP transport construction/injection.

pub fn adapt(content: &mut String) -> Result<(), Box<dyn std::error::Error>> {
    // Checked source adapter; generated artifacts are never patched by hand.
    if !content.contains("Debug") {
        return Err("upstream DTO derives changed; review secret protection".into());
    }
    *content = content.replace("Debug, ", "").replace(", Debug", "");
    let names: Vec<_> = content
        .lines()
        .filter_map(|line| {
            line.strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("pub enum "))
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned)
        })
        .collect();
    if names.is_empty() {
        return Err("upstream DTO declarations changed; review secret protection".into());
    }
    for name in names {
        content.push_str(&format!("\nimpl std::fmt::Debug for {name} {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str(\"{name}([REDACTED])\") }} }}\n"));
    }
    Ok(())
}

/// Checked client exception protection, independent of transport injection.
/// Success bodies are decoded normally but never retained in error envelopes.
pub fn adapt_client(
    source: &mut String,
    operations: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let derive = "#[derive(Debug, Clone)]\npub struct ApiError<E>";
    let decode = "format!(\"failed to deserialize 2xx response body: {}\", e)";
    let body = "body: body_text,";
    if source.matches(derive).count() != 1
        || source.matches(decode).count() != operations
        || source.matches("raw_body,").count() != operations * 3
        || source.matches(body).count() != operations * 3
    {
        return Err("upstream error envelope changed; review secret exception adapter".into());
    }
    let start = source
        .find("const API_ERROR_BODY_DISPLAY_LIMIT")
        .ok_or("missing error body formatter")?;
    let end = source[start..]
        .find("impl<E> ApiError<E> {")
        .map(|n| start + n)
        .ok_or("missing error helpers")?;
    let display_start = source
        .find("impl<E: std::fmt::Debug> std::fmt::Display for ApiError<E> {")
        .ok_or("missing ApiError Display")?;
    let display_end = source[display_start..]
        .find("impl<E: std::fmt::Debug> std::error::Error for ApiError<E> {}")
        .map(|n| display_start + n)
        .ok_or("missing ApiError Error impl")?;
    let mut output = source.clone();
    let mut arms = Vec::new();
    for (position, _) in output.match_indices(decode) {
        arms.push(
            output[..position]
                .rfind("Err(e)")
                .ok_or("missing decode error arm")?,
        );
    }
    if arms.windows(2).any(|a| a[0] >= a[1]) {
        return Err("ambiguous decode error arms".into());
    }
    // Rename only success decode arms; unreachable documented error decoders
    // remain behind status-first rejection and may still use their own e.
    // Apply after region replacements below so offsets remain original.
    output.replace_range(
        display_start..display_end,
        r#"impl<E> std::fmt::Debug for ApiError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiError").field("status", &self.status)
            .field("category", &self.category()).finish_non_exhaustive()
    }
}
impl<E> std::fmt::Display for ApiError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "API error {} ({})", self.status, self.category())
    }
}
"#,
    );
    output.replace_range(start..end, "");
    output=output.replace(derive,"#[derive(Clone)]\npub struct ApiError<E>")
        .replace(body,"body: String::new(),")
        .replace("raw_body,","raw_body: Vec::new(),")
        .replace("/// failures, the bounded body and parsing details are retained.","/// failures, raw bodies and parse values are discarded; only a safe category remains.")
        .replace("    /// Bounded 2xx response bytes; empty for non-2xx responses (body skipped).","    /// Always empty: response bytes must not escape through exceptions.");
    output=output.replace("impl<E> ApiError<E> {",r#"impl<E> ApiError<E> {
    /// Stable error classification without raw body, parse values or headers.
    pub fn category(&self) -> &'static str {
        if !(200..300).contains(&self.status) { "http_rejection" }
        else if self.parse_error.as_deref() == Some("success_response_decode") { "success_response_decode" }
        else { "unexpected_success_status" }
    }
"#);
    for _ in 0..operations {
        let position = output
            .find(decode)
            .ok_or("missing success decode expression")?;
        let arm = output[..position]
            .rfind("Err(e)")
            .ok_or("missing success decode arm")?;
        output.replace_range(
            position..position + decode.len(),
            "\"success_response_decode\".to_owned()",
        );
        output.replace_range(arm..arm + "Err(e)".len(), "Err(_e)");
    }
    // Verify that no success/error envelope can retain body bytes. The only
    // remaining raw_body variable is the local bounded input to deserialization.
    if output.contains("raw_body,") {
        return Err("unadapted response bytes in error envelope".into());
    }
    *source = output;
    Ok(())
}
