//! obscura-js (rquickjs port) — skeleton entry.
//! Mirrors the original public surface (see original src/lib.rs) minus V8-specific modules
//! (v8_flags, v8_lock removed; cdp_watchdog reimplemented via interrupt handler).

pub mod state;
pub mod bridge;
pub mod runtime;
pub mod markdown;      // keep as-is from original (pure JS/data)
// pub mod module_loader;  // port after bootstrap works (rquickjs Loader/Resolver)

pub use runtime::ObscuraRuntime;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boots_and_evals() {
        // NOTE: needs js/bootstrap.js present next to this crate. This test is the Phase 2
        // step-1 gate: runtime constructs and a trivial eval works.
        let rt = ObscuraRuntime::new().expect("runtime");
        rt.exec("globalThis.__probe = 1 + 1;").expect("exec");
    }
}
