//! Transient bounded observations. A cursor never survives a daemon session.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub daemon_id: Uuid,
    pub sequence: u64,
}

mod hub {
    use super::*;
    use crate::{Error, Result};
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Condvar, Mutex},
        time::Duration,
    };

    const MAX_EVENTS: usize = 256;
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    pub struct Hub {
        state: Mutex<State>,
        changed: Condvar,
    }
    struct State {
        cursor: Cursor,
        events: VecDeque<(u64, Value, usize)>,
        bytes: usize,
    }
    impl Hub {
        pub fn new() -> Self {
            Self {
                state: Mutex::new(State {
                    cursor: Cursor {
                        daemon_id: Uuid::new_v4(),
                        sequence: 0,
                    },
                    events: VecDeque::new(),
                    bytes: 0,
                }),
                changed: Condvar::new(),
            }
        }
        pub fn cursor(&self) -> Cursor {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cursor
                .clone()
        }
        pub fn publish(&self, kind: &str, installation_id: Option<Uuid>, data: Value) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.cursor.sequence == u64::MAX {
                state.cursor = Cursor {
                    daemon_id: Uuid::new_v4(),
                    sequence: 0,
                };
                state.events.clear();
                state.bytes = 0;
            }
            state.cursor.sequence += 1;
            let mut event = json!({"cursor":state.cursor,"kind":kind,"installation_id":installation_id,"data":data});
            let mut bytes = serde_json::to_vec(&event).map_or(usize::MAX, |b| b.len());
            if bytes > 256 * 1024 {
                event = json!({"cursor":state.cursor,"kind":"resync_required","installation_id":installation_id,"data":null});
                bytes = 256;
            }
            let sequence = state.cursor.sequence;
            state.events.push_back((sequence, event, bytes));
            state.bytes += bytes;
            while state.events.len() > MAX_EVENTS || state.bytes > MAX_BYTES {
                state.bytes -= state.events.pop_front().unwrap().2;
            }
            self.changed.notify_all();
        }
        fn validate(state: &State, cursor: &Cursor) -> Result<()> {
            let floor = state
                .events
                .front()
                .map_or(state.cursor.sequence, |e| e.0 - 1);
            if cursor.daemon_id != state.cursor.daemon_id
                || cursor.sequence < floor
                || cursor.sequence > state.cursor.sequence
            {
                return Err(Error::new(
                    "EVENT_CURSOR_EXPIRED",
                    "Read a new status snapshot before subscribing again",
                ));
            }
            Ok(())
        }
        pub fn after(&self, cursor: &Cursor, timeout: Duration) -> Result<Vec<Value>> {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            Self::validate(&state, cursor)?;
            if state.cursor.sequence == cursor.sequence && !timeout.is_zero() {
                state = self
                    .changed
                    .wait_timeout(state, timeout)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                Self::validate(&state, cursor)?;
            }
            Ok(state
                .events
                .iter()
                .filter(|e| e.0 > cursor.sequence)
                .take(16)
                .map(|e| e.1.clone())
                .collect())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cursor_replay_expiry_size_limits_and_session_identity() {
            let hub = Hub::new();
            let before = hub.cursor();
            hub.publish("instance", None, json!({"state":"RUNNING"}));
            assert_eq!(
                hub.after(&before, Duration::ZERO).unwrap()[0]["data"]["state"],
                "RUNNING"
            );
            let other = Hub::new().cursor();
            assert_eq!(
                hub.after(&other, Duration::ZERO).unwrap_err().code,
                "EVENT_CURSOR_EXPIRED"
            );
            for _ in 0..256 {
                hub.publish("change", None, Value::Null);
            }
            assert_eq!(
                hub.after(&before, Duration::ZERO).unwrap_err().code,
                "EVENT_CURSOR_EXPIRED"
            );
            let now = hub.cursor();
            hub.publish("instance", None, json!("a".repeat(300000)));
            assert_eq!(
                hub.after(&now, Duration::ZERO).unwrap()[0]["kind"],
                "resync_required"
            );
            assert!(hub.after(&hub.cursor(), Duration::ZERO).unwrap().is_empty());
        }
    }
}
pub(crate) use hub::Hub;
