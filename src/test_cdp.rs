fn test_event() {
    let _ = crate::cdp::browser_protocol::target::EventAttachedToTarget {
        session_id: Default::default(),
        target_info: Default::default(),
        waiting_for_debugger: Default::default(),
    };
}
