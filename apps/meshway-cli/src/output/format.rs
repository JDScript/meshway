//! Human formats of exact values: no float rounding of measured microseconds.
use chrono::{DateTime, Utc};

/// Exact microseconds as milliseconds with two decimals (truncated).
pub(crate) fn millis(us: u64) -> String {
    format!("{}.{:02} ms", us / 1000, us % 1000 / 10)
}

/// A coarse span such as `23h 59m`, `4m 12s` or `9s`.
pub(crate) fn span(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (days, hours, minutes, rest) = (
        seconds / 86_400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {rest}s")
    } else {
        format!("{rest}s")
    }
}

pub(crate) fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    format!("{} ago", span((now - at).num_seconds()))
}

pub(crate) fn left(until: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (until - now).num_seconds();
    if seconds <= 0 {
        "ended".into()
    } else {
        format!("{} left", span(seconds))
    }
}

/// camelCase wire names as lowercase words: `awaitingApproval` → `awaiting approval`.
pub(crate) fn words(value: impl serde::Serialize) -> String {
    let wire = serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut text = String::new();
    for character in wire.chars() {
        if character.is_ascii_uppercase() {
            text.push(' ');
            text.push(character.to_ascii_lowercase());
        } else {
            text.push(character);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_rtt_and_spans() {
        assert_eq!(millis(140), "0.14 ms");
        assert_eq!(millis(5123), "5.12 ms");
        assert_eq!(millis(120_009), "120.00 ms");
        assert_eq!(span(86_399), "23h 59m");
        assert_eq!(span(252), "4m 12s");
        assert_eq!(span(-3), "0s");
        assert_eq!(span(90_000), "1d 1h");
        assert_eq!(
            words(meshway_protocol::ipc::Phase::AwaitingApproval),
            "awaiting approval"
        );
    }
}
