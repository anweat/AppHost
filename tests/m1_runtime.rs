use apphost::runtime::{InstanceStatus, State};

#[test]
fn collector_failure_cannot_recover_from_later_readiness() {
    for ready in [false, true] {
        let mut status = InstanceStatus::default();
        if ready {
            status.all_ready();
        }
        status.collector_exited(false);
        status.all_ready();
        assert_eq!(status.state(), State::Failed);
        assert!(status.log_incomplete);
        assert_eq!(status.failure.as_deref(), Some("LOG_COLLECTION_FAILED"));
        assert!(!status.finish_stop(true));
        status.request_stop();
        assert!(!status.finish_stop(false));
        assert!(status.finish_stop(true));
        assert!(status.log_incomplete);
    }
}

#[test]
fn normal_eof_and_stop_race_preserve_user_intent_and_original_failure() {
    let mut status = InstanceStatus::default();
    status.collector_exited(true);
    status.all_ready();
    assert_eq!(status.state(), State::Running);
    assert!(!status.log_incomplete);
    status.fail("EARLY_EXIT");
    status.request_stop();
    status.collector_exited(false);
    assert_eq!(status.state(), State::Stopping);
    assert_eq!(status.failure.as_deref(), Some("EARLY_EXIT"));
    assert!(status.finish_stop(true));
    status.collector_exited(false);
    status.all_ready();
    assert_eq!(status.state(), State::Stopped);
}
