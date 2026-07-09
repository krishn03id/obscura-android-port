// search_cli: takes a query as argument, outputs JSON results to stdout
use std::time::Duration;
use obscura_js::runtime::ObscuraJsRuntime;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let query: String = std::env::args().nth(1).unwrap_or_default();
    if query.is_empty() {
        eprintln!("Usage: search_cli <query>");
        std::process::exit(1);
    }

    let mut rt = ObscuraJsRuntime::new();

    rt.execute_script("search-kickoff", &format!(
        r#"
        globalThis.__searchDone = false;
        globalThis.__searchResult = null;
        globalThis.__searchError = null;
        Deno.core.ops.op_web_search({}).then(function(r) {{
            globalThis.__searchResult = r;
            globalThis.__searchDone = true;
        }}).catch(function(e) {{
            globalThis.__searchError = e.message;
            globalThis.__searchDone = true;
        }});
        "#,
        serde_json::to_string(&query).unwrap()
    )).expect("exec failed");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        rt.run_event_loop().await.ok();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let done = rt.evaluate("globalThis.__searchDone").unwrap_or_default();
        if done == serde_json::Value::Bool(true) {
            let error = rt.evaluate("globalThis.__searchError").unwrap_or_default();
            if !error.is_null() {
                println!(r#"{{"error":"{}"}}"#, error.as_str().unwrap_or("unknown").replace('"', "\\\""));
            } else {
                let result = rt.evaluate("globalThis.__searchResult").unwrap_or_default();
                // op_web_search returns a JSON string; eval_json may parse it
                // back to a Value. Handle both cases.
                if let Some(s) = result.as_str() {
                    if s.starts_with('[') {
                        println!("{}", s);
                    } else {
                        println!(r#"{{"error":"unexpected result"}}"#);
                    }
                } else {
                    println!("{}", serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string()));
                }
            }
            break;
        }

        if std::time::Instant::now() >= deadline {
            println!(r#"{{"error":"timeout after 30s"}}"#);
            break;
        }
    }
}
