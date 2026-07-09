// metasearch_cli: multi-provider metasearch — queries DDG + Bing + Wikipedia
// in parallel, aggregates, deduplicates, ranks, and outputs JSON.
//
// Usage: metasearch_cli "search query"
// Output: JSON array of {title, url, snippet, source, score}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let query: String = std::env::args().nth(1).unwrap_or_default();
    if query.is_empty() {
        eprintln!("Usage: metasearch_cli <query>");
        std::process::exit(1);
    }

    match obscura_js::ops::metasearch(&query).await {
        Ok(results_json) => {
            println!("{}", results_json);
        }
        Err(e) => {
            eprintln!("Metasearch error: {}", e);
            println!("[]");
            std::process::exit(1);
        }
    }
}
