//! bridge.rs — installs the `Deno.core.ops` shim so bootstrap.js runs unmodified.
//!
//! This is the single most important file: it recreates the exact JS-facing shape that
//! bootstrap.js expects (`globalThis.Deno.core.ops.op_*`), backed by Rust functions.
//!
//! STATUS: skeleton. Ops are stubbed. Fill each in per porting/ops-inventory.md.

use std::sync::{Arc, Mutex};
use rquickjs::{Ctx, Object, function::Func};

use crate::state::ObscuraState;

/// Install `globalThis.Deno.core.ops = { ...all 22 ops... }` into the given context.
pub fn install_bridge(ctx: &Ctx<'_>, state: Arc<Mutex<ObscuraState>>) -> rquickjs::Result<()> {
    let global = ctx.globals();

    let deno = Object::new(ctx.clone())?;
    let core = Object::new(ctx.clone())?;
    let ops = Object::new(ctx.clone())?;

    // ---- PURE ops (no state) ----
    ops.set("op_url_parse", Func::from(op_url_parse))?;
    ops.set("op_url_resolve", Func::from(op_url_resolve))?;
    ops.set("op_encoding_for_label", Func::from(op_encoding_for_label))?;
    ops.set("op_random_bytes", Func::from(op_random_bytes))?;
    // TODO: op_url_set, op_url_encode_query, op_text_decode,
    //       op_subtle_{digest,hmac,aes_gcm,aes_cbc,aes_ctr,pbkdf2,hkdf}

    // ---- STATE ops (capture Arc<Mutex<ObscuraState>>) ----
    {
        let st = state.clone();
        ops.set("op_console_msg", Func::from(move |level: String, msg: String| {
            let _ = &st;
            match level.as_str() {
                "warn"  => tracing::warn!(target:"obscura::console","{msg}"),
                "error" => tracing::error!(target:"obscura::console","{msg}"),
                _       => tracing::info!(target:"obscura::console","{msg}"),
            }
        }))?;
    }
    {
        let st = state.clone();
        ops.set("op_binding_called", Func::from(move |name: String, payload: String| {
            st.lock().unwrap().pending_binding_calls.push((name, payload));
        }))?;
    }
    // TODO: op_dom, op_dom_inner, op_get_cookies, op_set_cookie, op_navigate

    // ---- ASYNC ops ----
    // op_fetch_url must be installed on an AsyncContext; see runtime.rs. It is left writable
    // because bootstrap.js reassigns it for request interception.
    // TODO: install op_fetch_url as an async Func on the async context.

    core.set("ops", ops)?;
    // A few bootstrap call sites use Deno.core.<helper> directly; add shims as needed:
    // core.set("encode", Func::from(...))?; etc. Enumerate with:
    //   grep -oE 'Deno\.core\.[a-zA-Z.]+' js/bootstrap.js | sort -u
    deno.set("core", core)?;
    global.set("Deno", deno)?;
    Ok(())
}

// ---------- stub op bodies (replace with real logic from ops.rs) ----------

fn op_url_parse(input: String) -> rquickjs::Result<String> {
    // real version returns a serde struct; stub returns the href
    let u = url::Url::parse(&input).map_err(|e| rquickjs::Error::new_from_js_message("url","parse",e.to_string()))?;
    Ok(u.to_string())
}
fn op_url_resolve(base: String, rel: String) -> rquickjs::Result<String> {
    let b = url::Url::parse(&base).map_err(|e| rquickjs::Error::new_from_js_message("url","parse",e.to_string()))?;
    Ok(b.join(&rel).map_err(|e| rquickjs::Error::new_from_js_message("url","join",e.to_string()))?.to_string())
}
fn op_encoding_for_label(label: String) -> String {
    encoding_rs::Encoding::for_label(label.as_bytes())
        .map(|e| e.name().to_string())
        .unwrap_or_default()
}
fn op_random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    getrandom::getrandom(&mut v).expect("rng");
    v
}
