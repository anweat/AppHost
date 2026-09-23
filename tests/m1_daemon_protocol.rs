use apphost::daemon::protocol::{self, MAX_REQUEST, VERSION};
use serde_json::{Value, json};

fn request() -> Value {
    json!({"version":VERSION,"request_id":"request-1","method":"hello","params":{}})
}

#[test]
fn actor_is_an_optional_strict_attribution_not_an_authority() {
    use apphost::journal::Actor;
    let decode = |v: &Value| protocol::decode(&serde_json::to_vec(v).unwrap());
    assert_eq!(decode(&request()).unwrap().actor, Actor::Human);
    for (name, actor) in [
        ("human", Actor::Human),
        ("agent", Actor::Agent),
        ("system", Actor::System),
    ] {
        let mut v = request();
        v["actor"] = json!(name);
        assert_eq!(decode(&v).unwrap().actor, actor);
    }
    for bad in [json!("admin"), json!("Agent"), json!(null), json!(1)] {
        let mut v = request();
        v["actor"] = bad;
        assert_eq!(decode(&v).unwrap_err().code, "INVALID_REQUEST");
    }
}

#[test]
fn envelopes_reject_unknown_fields_bad_types_and_unbounded_ids() {
    for (field, value) in [
        ("extra", json!(true)),
        ("request_id", json!("")),
        ("request_id", json!("a".repeat(129))),
        ("request_id", json!("newline\n")),
        ("request_id", json!("非 ASCII")),
        ("method", json!("a".repeat(65))),
        ("params", json!([])),
        ("version", json!(-1)),
    ] {
        let mut request = request();
        request[field] = value;
        assert_eq!(
            protocol::decode(&serde_json::to_vec(&request).unwrap())
                .unwrap_err()
                .code,
            "INVALID_REQUEST"
        );
    }
    for bytes in [b"null".as_slice(), b"{", b"\xff", b"{}"] {
        assert_eq!(protocol::decode(bytes).unwrap_err().code, "INVALID_REQUEST");
    }
    assert_eq!(
        protocol::decode(&vec![b' '; MAX_REQUEST + 1])
            .unwrap_err()
            .code,
        "REQUEST_TOO_LARGE"
    );
    let mut bytes = serde_json::to_vec(&request()).unwrap();
    bytes.resize(MAX_REQUEST, b' ');
    assert!(protocol::decode(&bytes).is_ok());
}

#[test]
fn version_methods_and_params_fail_before_accessing_a_registry() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("absent");
    let mut request = protocol::decode(&serde_json::to_vec(&request()).unwrap()).unwrap();
    request.version = 2;
    assert_eq!(
        protocol::dispatch(&data, &request, &json!({}))
            .unwrap_err()
            .code,
        "PROTOCOL_VERSION_UNSUPPORTED"
    );
    request.version = 1;
    request.method = "start".into();
    assert_eq!(
        protocol::dispatch(&data, &request, &json!({}))
            .unwrap_err()
            .code,
        "METHOD_NOT_FOUND"
    );
    request.method = "check-launch".into();
    for params in [
        json!({}),
        json!({"installation_id":uuid::Uuid::new_v4(),"extra":true}),
        json!({"installation_id":"invalid"}),
    ] {
        request.params = params;
        assert_eq!(
            protocol::dispatch(&data, &request, &json!({}))
                .unwrap_err()
                .code,
            "INVALID_PARAMS"
        );
    }
    assert!(!data.exists());
}

#[test]
fn response_limits_correlation_and_exclusive_outcomes_are_checked() {
    let response = protocol::response(Some("request-1"), Ok(json!({"ok":true})));
    let exact = serde_json::to_vec(&response).unwrap().len();
    assert_eq!(protocol::encode(&response, exact).unwrap().len(), exact + 1);
    assert_eq!(
        protocol::encode(&response, exact - 1).unwrap_err().code,
        "MESSAGE_TOO_LARGE"
    );
    let bytes = protocol::encode(&response, exact).unwrap();
    assert_eq!(
        protocol::result(&bytes, "request-1").unwrap(),
        json!({"ok":true})
    );
    assert_eq!(
        protocol::result(&bytes, "other").unwrap_err().code,
        "PROTOCOL_ERROR"
    );
    let mut response = response;
    response["error"] = json!({"code":"FAIL","message":"failed","details":null});
    assert_eq!(
        protocol::result(&serde_json::to_vec(&response).unwrap(), "request-1")
            .unwrap_err()
            .code,
        "PROTOCOL_ERROR"
    );
    let response = protocol::response(
        Some("request-1"),
        Err(apphost::Error::new("REVIEW_REQUIRED", "Review declaration")),
    );
    assert_eq!(
        protocol::result(&serde_json::to_vec(&response).unwrap(), "request-1")
            .unwrap_err()
            .code,
        "REVIEW_REQUIRED"
    );
}

#[test]
fn null_success_is_valid_but_two_outcome_keys_are_never_valid() {
    let response = protocol::response(Some("null-result"), Ok(Value::Null));
    assert_eq!(
        protocol::result(&serde_json::to_vec(&response).unwrap(), "null-result").unwrap(),
        Value::Null
    );
    let mut both = response;
    both["error"] = Value::Null;
    assert_eq!(
        protocol::result(&serde_json::to_vec(&both).unwrap(), "null-result")
            .unwrap_err()
            .code,
        "PROTOCOL_ERROR"
    );
}
