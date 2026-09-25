//! Feed JSONL captured by gateway_navigation_probe.py into the actual policy.
use serde::Deserialize;
use std::io::{self, BufRead};

#[derive(Deserialize)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in io::stdin().lock().lines() {
        let r: Request = serde_json::from_str(&line?)?;
        let headers: Vec<_> = r
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        println!(
            "{}",
            serde_json::json!({"path":r.path,"navigation_eligible":apphost::gateway::is_wake_navigation(&r.method,&headers)})
        );
    }
    Ok(())
}
