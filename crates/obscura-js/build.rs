// No V8 snapshot — bootstrap.js is embedded as a string and eval'd at runtime startup.
fn main() {
    println!("cargo:rerun-if-changed=js/bootstrap.js");
    println!("cargo:rerun-if-changed=build.rs");
}
