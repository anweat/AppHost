use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::Path;
use uuid::Uuid;

pub const VERSION: u32 = 1;
pub const MAX_REQUEST: usize = 64 * 1024;
pub const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub method: String,
    pub params: Value,
    #[serde(default, skip_serializing_if = "crate::journal::Actor::is_human")]
    pub actor: crate::journal::Actor,
}

pub fn decode(bytes: &[u8]) -> Result<Request> {
    if bytes.len() > MAX_REQUEST {
        return Err(Error::new(
            "REQUEST_TOO_LARGE",
            "Daemon request exceeds 64 KiB",
        ));
    }
    let request: Request = serde_json::from_slice(bytes)
        .map_err(|_| Error::new("INVALID_REQUEST", "Invalid daemon request envelope"))?;
    for (text, limit) in [(&request.request_id, 128), (&request.method, 64)] {
        if text.is_empty() || text.len() > limit || !text.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Error::new(
                "INVALID_REQUEST",
                "Invalid request_id or method",
            ));
        }
    }
    if !request.params.is_object() {
        return Err(Error::new("INVALID_REQUEST", "params must be an object"));
    }
    Ok(request)
}

pub fn dispatch(data_dir: &Path, request: &Request, hello: &Value) -> Result<Value> {
    if request.version != VERSION {
        return Err(Error::new(
            "PROTOCOL_VERSION_UNSUPPORTED",
            "Daemon protocol version 1 is required",
        ));
    }
    match request.method.as_str() {
        "hello"
            if request
                .params
                .as_object()
                .is_some_and(|params| params.is_empty()) =>
        {
            Ok(hello.clone())
        }
        "hello" => Err(Error::new("INVALID_PARAMS", "hello accepts no parameters")),
        "check-launch" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Params {
                installation_id: Uuid,
            }
            let params: Params = serde_json::from_value(request.params.clone()).map_err(|_| {
                Error::new("INVALID_PARAMS", "Expected only installation_id as a UUID")
            })?;
            let plan = crate::runtime::LaunchPlan::prepare(data_dir, params.installation_id)?;
            serde_json::to_value(plan.summary())
                .map_err(|_| Error::new("INTERNAL_ERROR", "Cannot encode launch summary"))
        }
        _ => Err(Error::new(
            "METHOD_NOT_FOUND",
            "Daemon method is not available",
        )),
    }
}

pub fn response(request_id: Option<&str>, result: Result<Value>) -> Value {
    match result {
        Ok(result) => json!({"version":VERSION,"request_id":request_id,"result":result}),
        Err(error) => json!({"version":VERSION,"request_id":request_id,"error":error}),
    }
}

// Cap serialization itself, rather than allocate an arbitrary response and test
// its size afterwards. This also keeps error handling on the same wire limit.
pub fn encode(value: &Value, maximum: usize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        maximum: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("message limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        maximum,
    };
    serde_json::to_writer(&mut output, value)
        .map_err(|_| Error::new("MESSAGE_TOO_LARGE", "Daemon message exceeds its byte limit"))?;
    output.bytes.push(b'\n');
    Ok(output.bytes)
}

pub fn result(bytes: &[u8], request_id: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireError {
        code: String,
        message: String,
        details: Value,
    }
    #[derive(Deserialize)]
    #[serde(untagged, deny_unknown_fields)]
    enum Response {
        Success {
            version: u32,
            request_id: Option<String>,
            result: Value,
        },
        Failure {
            version: u32,
            request_id: Option<String>,
            error: WireError,
        },
    }
    if bytes.len() > MAX_RESPONSE {
        return Err(Error::new(
            "MESSAGE_TOO_LARGE",
            "Daemon response exceeds 1 MiB",
        ));
    }
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|_| Error::new("PROTOCOL_ERROR", "Invalid daemon response"))?;
    let (version, id, outcome) = match response {
        Response::Success {
            version,
            request_id,
            result,
        } => (version, request_id, Ok(result)),
        Response::Failure {
            version,
            request_id,
            error,
        } => (
            version,
            request_id,
            Err(Error::new(&error.code, error.message).details(error.details)),
        ),
    };
    if version != VERSION {
        return Err(Error::new(
            "PROTOCOL_VERSION_UNSUPPORTED",
            "Incompatible daemon response version",
        ));
    }
    if id.as_deref() != Some(request_id) {
        return Err(Error::new(
            "PROTOCOL_ERROR",
            "Daemon response request_id mismatch",
        ));
    }
    outcome
}
