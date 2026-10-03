/// ECMA-262 compatible expression for canonical decimal integers in 1..=max.
/// This keeps string duration schemas as strict as checked integer conversion.
pub(crate) fn positive_decimal_pattern(max: u64) -> String {
    let digits = max.to_string();
    let mut alternatives = Vec::new();
    if digits.len() > 1 {
        alternatives.push(format!("[1-9][0-9]{{0,{}}}", digits.len() - 2));
    }
    for (index, digit) in digits.bytes().enumerate() {
        let first = if index == 0 { b'1' } else { b'0' };
        if digit > first {
            let prefix = &digits[..index];
            let lower = char::from(first);
            let upper = char::from(digit - 1);
            let suffix = digits.len() - index - 1;
            alternatives.push(format!("{prefix}[{lower}-{upper}][0-9]{{{suffix}}}"));
        }
    }
    alternatives.push(digits);
    format!("(?:{})", alternatives.join("|"))
}
