#[macro_use]
extern crate html5ever;

pub mod cdp_watchdog;
pub mod module_loader;
pub mod runtime;
pub mod ops;
pub mod state;
pub mod v8_lock;
pub mod markdown;

pub use markdown::HTML_TO_MARKDOWN_JS;

/// No-op: V8 flags don't exist in QuickJS. Kept for API compatibility with
/// obscura-cli which calls `obscura_js::set_v8_flags(...)`.
pub fn set_v8_flags(_flags: &str) {}
