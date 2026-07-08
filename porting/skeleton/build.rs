//! No V8 snapshot here (deno_core baked one via create_snapshot).
//! QuickJS-NG has no V8-style snapshot; bootstrap.js is embedded and run at
//! runtime startup instead. We just embed the JS as a string.
use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=js/bootstrap.js");
    let out = env::var("OUT_DIR").unwrap();
    // Fallback stub so the skeleton compiles before you copy the real 366 KB file.
    let src = Path::new("js/bootstrap.js");
    let js = fs::read_to_string(src)
        .unwrap_or_else(|_| "globalThis.__bootstrap_ok = true;\n".to_string());
    fs::write(Path::new(&out).join("bootstrap.js"), js).unwrap();
}
