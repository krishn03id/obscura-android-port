//! Phase 3 JNI boundary stub for the Kotlin/Android app.
//! Add to a cdylib crate: crate-type = ["cdylib"], deps: jni = "0.21".
//! Build per-ABI, drop .so into app/src/main/jniLibs/<abi>/libobscura.so
//!
//! Threading assumption (per Obscura architecture): the QuickJS Runtime is
//! single-threaded and NOT Send-safe across arbitrary threads. Hold it inside
//! a foreground Service on ONE dedicated worker thread; JNI calls from Kotlin
//! post messages to that thread rather than touching the Runtime directly.
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

/// Kotlin: external fun nativeEval(script: String): String
#[no_mangle]
pub extern "system" fn Java_com_obscura_Engine_nativeEval(
    mut env: JNIEnv,
    _class: JClass,
    script: JString,
) -> jstring {
    let script: String = match env.get_string(&script) {
        Ok(s) => s.into(),
        Err(_) => return env.new_string("ERR: bad input").unwrap().into_raw(),
    };
    // TODO: forward `script` to the engine worker thread (channel), await result.
    // let result = ENGINE.eval_on_worker(script);
    let result = format!("eval stub received {} bytes", script.len());
    env.new_string(result).unwrap().into_raw()
}

/// Kotlin: external fun nativeStartCdp(port: Int): Int  (returns actual port)
#[no_mangle]
pub extern "system" fn Java_com_obscura_Engine_nativeStartCdp(
    _env: JNIEnv,
    _class: JClass,
    port: i32,
) -> i32 {
    // TODO: spawn obscura-cdp websocket server bound to 127.0.0.1:port on the
    // engine's tokio runtime; return the bound port.
    port
}
