// Quick bootstrap test: create the runtime, eval bootstrap.js, check globals exist.
fn main() {
    println!("Creating ObscuraJsRuntime...");
    let mut rt = obscura_js::runtime::ObscuraJsRuntime::new();
    println!("Runtime created OK");

    println!("Evaluating 1+1...");
    match rt.evaluate("1 + 1") {
        Ok(v) => println!("  1+1 = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    println!("Checking window global...");
    match rt.evaluate("typeof window") {
        Ok(v) => println!("  typeof window = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    println!("Checking document global...");
    match rt.evaluate("typeof document") {
        Ok(v) => println!("  typeof document = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    println!("Checking console global...");
    match rt.evaluate("typeof console") {
        Ok(v) => println!("  typeof console = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    println!("Checking console.log...");
    match rt.evaluate("typeof console.log") {
        Ok(v) => println!("  typeof console.log = {}", v),
        Err(e) => println!("  ERROR: {}", e),
    }

    println!("DONE");
}
