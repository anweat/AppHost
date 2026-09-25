//! A caller-scoped observation, never an automatic retry or durable workflow.
use super::*;
impl Manager {
    pub(crate) fn port_condition(&self, id: Uuid, failure: Uuid) -> Result<Value> {
        let value = crate::failure::read(&self.data, id, None)?;
        let current = &value["failure"];
        if current["failure_id"] != json!(failure) {
            return Err(Error::new(
                "CONDITION_EXPIRED",
                "Failure has been replaced or its evidence expired",
            ));
        }
        let at = current["at_ms"].as_u64().unwrap_or(0);
        if value["history"]["last_success_at_ms"]
            .as_u64()
            .is_some_and(|success| success >= at)
        {
            return Err(Error::new(
                "CONDITION_EXPIRED",
                "Installation has already succeeded",
            ));
        }
        if current["cause_code"] != "PORT_IN_USE" && current["evidence"]["code"] != "PORT_IN_USE" {
            return Err(Error::new(
                "CONDITION_UNAVAILABLE",
                "Only concrete port-conflict conditions can be watched",
            ));
        }
        let details = &current["details"];
        let port = details["details"]["port"]
            .as_u64()
            .or_else(|| details["operation_error"]["details"]["port"].as_u64())
            .or_else(|| {
                details["services"]
                    .as_array()?
                    .iter()
                    .find(|s| s["name"] == current["service"])?["port"]
                    .as_u64()
            })
            .filter(|p| (1..=65535).contains(p))
            .ok_or_else(|| {
                Error::new(
                    "CONDITION_UNAVAILABLE",
                    "No declared port is recorded in this failure",
                )
            })? as u16;
        let (state, error) = match port_available(port) {
            Ok(()) => ("cleared", Value::Null),
            Err(error) if error.code == "PORT_IN_USE" => ("blocked", Value::Null),
            Err(error) => ("unknown", json!(error.code)),
        };
        Ok(
            json!({"installation_id":id,"failure_id":failure,"code":"PORT_IN_USE","port":port,
            "state":state,"error_code":error,"observed_at_ms":now(),"complete":state=="cleared",
            "actions":if state=="cleared" { vec!["retry"] } else { vec![] }}),
        )
    }
}
