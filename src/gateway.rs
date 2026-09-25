//! Browser navigation admission only. This does not authorize a launch: the
//! gateway must also resolve a registered Host and pass runtime/trust checks.

/// Inspect raw header pairs so duplicate metadata cannot be silently collapsed
/// by an HTTP adapter. Values are tokens and deliberately case-sensitive.
pub fn is_wake_navigation(method: &str, headers: &[(&str, &str)]) -> bool {
    if method != "GET" || headers.len() > 128 {
        return false;
    }
    let names = [
        "sec-fetch-dest",
        "sec-fetch-mode",
        "sec-fetch-site",
        "sec-fetch-user",
        "sec-purpose",
        "purpose",
    ];
    let mut values = [None; 6];
    for (name, value) in headers {
        if let Some(index) = names.iter().position(|n| name.eq_ignore_ascii_case(n)) {
            if values[index].is_some()
                || value.len() > 256
                || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
            {
                return false;
            }
            values[index] = Some(value.trim_matches([' ', '\t']));
        }
    }
    if values[0] != Some("document")
        || values[1] != Some("navigate")
        || values[2] != Some("none")
        || values[3].is_some_and(|v| v != "?1")
    {
        return false;
    }
    // Safari may omit Sec-Fetch-User. Site=none still distinguishes browser UI
    // navigation from ordinary web-page initiators; never infer it from the UA.
    !values[4..].iter().flatten().any(|v| {
        v.split(|c: char| c.is_ascii_whitespace() || c == ';' || c == ',')
            .any(|token| {
                token.eq_ignore_ascii_case("prefetch") || token.eq_ignore_ascii_case("prerender")
            })
    })
}
