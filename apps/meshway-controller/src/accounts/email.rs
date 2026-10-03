use super::AccountError;

/// Deliberately supports unquoted ASCII dot-atom local parts only. We fold their
/// case as an account-identifier policy, not an assertion about SMTP delivery.
/// Domains use UTS #46/IDNA ASCII conversion and DNS hostname label constraints.
/// Unicode whitespace is trimmed only at the edges; controls are always invalid.
pub(super) fn normalize(input: &str) -> Result<String, AccountError> {
    let invalid = || AccountError::InvalidEmail;
    if input.len() > 1024 || input.chars().any(char::is_control) {
        return Err(invalid());
    }
    let (local, domain) = input.trim().split_once('@').ok_or_else(invalid)?;
    if local.is_empty()
        || local.len() > 64
        || !local.is_ascii()
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
    {
        return Err(invalid());
    }
    let domain = idna::domain_to_ascii_strict(domain)
        .map_err(|_| invalid())?
        .to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 253
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return Err(invalid());
    }
    let result = format!("{}@{domain}", local.to_ascii_lowercase());
    if result.len() > 254 {
        return Err(invalid());
    }
    Ok(result)
}
