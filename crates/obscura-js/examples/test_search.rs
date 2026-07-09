// Test: op_web_search
// Evaluates a search, pumps the event loop, prints results
use std::time::Duration;
use obscura_js::runtime::ObscuraJsRuntime;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut rt = ObscuraJsRuntime::new();

    // Kick off the search and store result in globalThis.__searchDone
    let search_js = r#"
        globalThis.__searchDone = false;
        globalThis.__searchResult = null;
        globalThis.__searchError = null;
        Deno.core.ops.op_web_search("what is cron job")
            .then(function(r) { globalThis.__searchResult = r; globalThis.__searchDone = true; })
            .catch(function(e) { globalThis.__searchError = e.message; globalThis.__searchDone = true; });
    "#;
    rt.execute_script("search-kickoff", search_js).expect("exec failed");

    // Pump the event loop for up to 30s
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        rt.run_event_loop().await.ok();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let done = rt.evaluate("globalThis.__searchDone").unwrap_or_default();
        if done == serde_json::Value::Bool(true) {
            let result = rt.evaluate("globalThis.__searchResult").unwrap_or_default();
            let error = rt.evaluate("globalThis.__searchError").unwrap_or_default();
            if !error.is_null() {
                println!("SEARCH_ERROR: {}", error);
            } else {
                println!("SEARCH_RESULTS: {}", serde_json::to_string_pretty(&result).unwrap_or_default());
            }
            break;
        }

        if std::time::Instant::now() >= deadline {
            println!("TIMEOUT after 30s");
            break;
        }
    }
}
