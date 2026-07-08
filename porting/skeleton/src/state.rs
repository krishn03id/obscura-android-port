//! state.rs — the shared state that ops read/write.
//!
//! Keep field names/types IDENTICAL to the original obscura-js SharedState/ObscuraState so
//! obscura-browser/obscura-cdp continue to compile unchanged. Re-read the real struct from
//! crates/obscura-js/src/ops.rs before finalizing (grep 'struct ObscuraState' / 'SharedState').

#[derive(Default)]
pub struct ObscuraState {
    /// binding calls queued from JS for the host to drain (op_binding_called)
    pub pending_binding_calls: Vec<(String, String)>,
    // TODO: cookies jar, current url/navigation state, DOM handle/registry, intercept tx, etc.
    //       Mirror the original exactly.
}
