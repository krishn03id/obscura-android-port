// Quick perf: just compute benchmarks, no external processes.
use std::time::Instant;

fn main() {
    println!("=== QuickJS-NG Compute Benchmark ===\n");
    let mut rt = obscura_js::runtime::ObscuraJsRuntime::new();

    let benches = [
        ("1M loop sum",      "var s=0; for(var i=0;i<1000000;i++){ s+=i; } s"),
        ("10K string concat", "var s=''; for(var i=0;i<10000;i++){ s+='x'; } s.length"),
        ("1K JSON roundtrip", r#"var a=[]; for(var i=0;i<1000;i++) a.push({id:i,n:"x"+i}); JSON.parse(JSON.stringify(a)).length"#),
        ("1K regex matches",  r#"var re=/(\d+)-(\d+)/g,s="123-456;789-012;"; var c=0; for(var i=0;i<1000;i++){re.lastIndex=0;while(re.exec(s))c++;} c"#),
        ("fib(20)",           "function f(n){return n<2?n:f(n-1)+f(n-2);} f(20)"),
        ("100K array sort",   "var a=[]; for(var i=0;i<100000;i++) a.push(Math.random()*1000); a.sort(function(a,b){return a-b}); a.length"),
    ];

    for (name, code) in &benches {
        let t0 = Instant::now();
        let v = rt.evaluate(code);
        let dt = t0.elapsed();
        match v {
            Ok(val) => println!("{:>30} = {:<10}  [{:5.0}ms]", name, val, dt.as_secs_f64() * 1000.0),
            Err(e)  => println!("{:>30} = ERR: {}    [{:5.0}ms]", name, e, dt.as_secs_f64() * 1000.0),
        }
    }

    // Real fetch timing
    println!("\n--- Real page fetch ---");
    for (name, url) in [("example.com", "https://example.com"), ("hackernews", "https://news.ycombinator.com")] {
        let t0 = Instant::now();
        let result = std::process::Command::new("./target/release/obscura")
            .arg("fetch").arg(url).output();
        let dt = t0.elapsed();
        match result {
            Ok(out) if out.status.success() => {
                let html = String::from_utf8_lossy(&out.stdout);
                println!("{:>30} [{:5.0}ms] ({} bytes)", name, dt.as_secs_f64() * 1000.0, html.len());
            }
            _ => println!("{:>30} FAILED [{:.0}ms]", name, dt.as_secs_f64() * 1000.0),
        }
    }
    println!("\nDone. QuickJS-NG = no JIT, interpreted only.");
}
