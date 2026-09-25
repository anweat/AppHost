use apphost::gateway::is_wake_navigation;

fn navigation() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Sec-Fetch-Dest", "document"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Site", "none"),
    ]
}

#[test]
fn browser_navigation_accepts_missing_user_without_ua_specific_rules() {
    let mut headers = navigation();
    assert!(is_wake_navigation("GET", &headers));
    headers.push(("Sec-Fetch-User", "?1"));
    assert!(is_wake_navigation("GET", &headers));
    headers.push(("User-Agent", "not a browser brand"));
    assert!(is_wake_navigation("GET", &headers));
    assert!(!is_wake_navigation("HEAD", &headers));
    assert!(!is_wake_navigation("POST", &headers));
    assert!(!is_wake_navigation("get", &headers));
}

#[test]
fn ordinary_page_initiators_subresources_and_missing_metadata_cannot_wake() {
    for i in 0..3 {
        let mut headers = navigation();
        headers.remove(i);
        assert!(!is_wake_navigation("GET", &headers));
    }
    for (i, value) in [
        (0, "iframe"),
        (0, "image"),
        (0, "script"),
        (0, "empty"),
        (1, "cors"),
        (1, "websocket"),
        (2, "same-origin"),
        (2, "same-site"),
        (2, "cross-site"),
    ] {
        let mut headers = navigation();
        headers[i].1 = value;
        headers.push(("Sec-Fetch-User", "?1"));
        assert!(!is_wake_navigation("GET", &headers), "{headers:?}");
    }
}

#[test]
fn explicit_speculative_loads_are_rejected_even_with_navigation_metadata() {
    for name in ["Sec-Purpose", "Purpose"] {
        for value in [
            "prefetch",
            "prefetch;prerender",
            "prefetch;anonymous-client-ip",
            "prerender",
            "PREFETCH",
            "prefetch, prerender",
        ] {
            let mut headers = navigation();
            headers.push((name, value));
            assert!(!is_wake_navigation("GET", &headers));
        }
    }
}

#[test]
fn conflicting_duplicate_and_malformed_metadata_fail_closed() {
    for (name, value) in [
        ("sec-fetch-site", "none"),
        ("SEC-FETCH-DEST", "image"),
        ("Sec-Fetch-User", "?0"),
        ("Sec-Fetch-User", "?1, ?1"),
        ("Sec-Fetch-User", "?1\r\nX:1"),
    ] {
        let mut headers = navigation();
        headers.push((name, value));
        assert!(!is_wake_navigation("GET", &headers));
    }
    let headers = [
        ("SEC-FETCH-DEST", " document\t"),
        ("Sec-Fetch-Mode", "navigate"),
        ("sec-fetch-site", "none"),
    ];
    assert!(is_wake_navigation("GET", &headers));
    let mut headers = navigation();
    headers.extend(std::iter::repeat_n(("other", "v"), 126));
    assert!(!is_wake_navigation("GET", &headers));
}

#[test]
fn header_budgets_and_optional_header_duplicates_are_enforced() {
    let mut headers = navigation();
    headers.extend(std::iter::repeat_n(("other", "v"), 125));
    assert!(is_wake_navigation("GET", &headers));
    headers.push(("other", "v"));
    assert!(!is_wake_navigation("GET", &headers));
    let oversized = "x".repeat(257);
    let mut headers = navigation();
    headers.push(("Purpose", &oversized));
    assert!(!is_wake_navigation("GET", &headers));
    for (name, value) in [
        ("Sec-Fetch-User", "?1"),
        ("Purpose", ""),
        ("Sec-Purpose", ""),
    ] {
        let mut headers = navigation();
        headers.extend([(name, value), (name, value)]);
        assert!(!is_wake_navigation("GET", &headers));
    }
    for value in ["NONE", "none, none", "none\0", "none\u{7f}"] {
        let mut headers = navigation();
        headers[2].1 = value;
        assert!(!is_wake_navigation("GET", &headers));
    }
}
