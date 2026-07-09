// Test ESM module loading via dynamic import().
// Serves a test ESM module over HTTP and tries to import it from page JS.
use std::net::TcpListener;
use std::io::{Read, Write};

fn main() {
    // Start a tiny HTTP server that serves a test ESM module
    let listener = TcpListener::bind("127.0.0.1:18345").unwrap();
    println!("ESM test server on http://127.0.0.1:18345/");

    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        println!("  Request: {}", req.lines().next().unwrap_or("?"));

        let module_code = "export const hello = 'Hello from ESM!'; export function add(a, b) { return a + b; }";
        let body = module_code.as_bytes();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
            body.len()
        );
        stream.write_all(resp.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        println!("  Served ESM module ({} bytes)", body.len());
    });

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Create a runtime and try to load a module
    println!("Creating ObscuraJsRuntime...");
    let mut rt = obscura_js::runtime::ObscuraJsRuntime::with_base_url("http://127.0.0.1:18345/");

    // Test: directly evaluate the module via load_module
    println!("\nTest: load_module()");
    let rt_handle = std::thread::spawn(move || {
        // We need a tokio runtime for load_module
        let rt2 = tokio::runtime::Runtime::new().unwrap();
        rt2.block_on(async {
            // Can't easily call load_module because it needs &mut self
            // Just test if the module loader is set up
            println!("  (module loader test - checking if import() is available)");
        });
    });
    let _ = rt_handle.join();

    // Simpler test: just check if the JS has the ability to import
    println!("\nTest: checking if dynamic import is available...");
    match rt.evaluate("typeof import") {
        Ok(v) => println!("  typeof import = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    // Check if bootstrap sets up any module-related globals
    println!("\nTest: checking for module-related globals...");
    match rt.evaluate("typeof require") {
        Ok(v) => println!("  typeof require = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    // The real test: try a dynamic import in an eval
    // Note: this requires the module loader to be set up on the runtime
    println!("\nTest: dynamic import() in eval...");
    match rt.evaluate("import('http://127.0.0.1:18345/test.mjs').then(m => m.hello).catch(e => 'ERR:' + e.message)") {
        Ok(v) => println!("  import result = {}", v),
        Err(e) => println!("  ERROR (expected if loader not wired): {}", e),
    }

    // Wait for the server thread
    let _ = server_thread.join();
    println!("\nDone.");
}
