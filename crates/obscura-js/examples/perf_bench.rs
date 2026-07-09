// Perf benchmark: JS compute speed + real page fetch timing.
use std::time::Instant;

fn main() {
    println!("=== Obscura (QuickJS-NG) Performance Benchmark ===\n");

    // --- Pure JS compute benchmarks ---
    let mut rt = obscura_js::runtime::ObscuraJsRuntime::new();

    // Benchmark 1: tight loop (1M iterations)
    print!("Bench 1: 1M loop sum          ");
    let t0 = Instant::now();
    let v = rt.evaluate("var s=0; for(var i=0;i<1000000;i++){ s+=i; } s").unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // Benchmark 2: string concatenation (10K iterations)
    print!("Bench 2: 10K string concat    ");
    let t0 = Instant::now();
    let v = rt.evaluate("var s=''; for(var i=0;i<10000;i++){ s+='x'; } s.length").unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // Benchmark 3: JSON parse + stringify (1K objects)
    print!("Bench 3: 1K JSON parse+string  ");
    let t0 = Instant::now();
    let v = rt.evaluate(r#"
        var arr = [];
        for (var i = 0; i < 1000; i++) arr.push({id:i, name:"item"+i, tags:[1,2,3]});
        var json = JSON.stringify(arr);
        var back = JSON.parse(json);
        back.length
    "#).unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // Benchmark 4: regex (1K matches)
    print!("Bench 4: 1K regex matches      ");
    let t0 = Instant::now();
    let v = rt.evaluate(r#"
        var re = /(\d+)-(\d+)/g;
        var s = "123-456;789-012;345-678;";
        var count = 0;
        for (var i = 0; i < 1000; i++) {
            var m;
            re.lastIndex = 0;
            while ((m = re.exec(s)) !== null) count++;
        }
        count
    "#).unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // Benchmark 5: array operations (sort 100K)
    print!("Bench 5: sort 100K array       ");
    let t0 = Instant::now();
    let v = rt.evaluate(r#"
        var a = [];
        for (var i = 0; i < 100000; i++) a.push(Math.random());
        a.sort(function(a,b){return a-b;});
        a.length
    "#).unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // Benchmark 6: function calls (1M calls)
    print!("Bench 6: 1M function calls     ");
    let t0 = Instant::now();
    let v = rt.evaluate(r#"
        function fib(n) { return n < 2 ? n : fib(n-1) + fib(n-2); }
        fib(20)
    "#).unwrap();
    let dt = t0.elapsed();
    println!("= {}  [{:.1}ms]", v, dt.as_millis());

    // --- Real page fetch timing ---
    println!("\n--- Real page fetch ---");

    // Simple page
    print!("Fetch example.com             ");
    let t0 = Instant::now();
    let result = std::process::Command::new("./target/release/obscura")
        .arg("fetch")
        .arg("https://example.com")
        .output();
    let dt = t0.elapsed();
    match result {
        Ok(out) if out.status.success() => {
            let html = String::from_utf8_lossy(&out.stdout);
            let len = html.lines().map(|l| l.len()).sum::<usize>();
            println!("[{:.0}ms] ({} bytes HTML)", dt.as_millis(), len);
        }
        _ => println!("FAILED [{:.0}ms]", dt.as_millis()),
    }

    // JS-heavy page (a real website)
    print!("Fetch hackernews (ycombinator) ");
    let t0 = Instant::now();
    let result = std::process::Command::new("./target/release/obscura")
        .arg("fetch")
        .arg("https://news.ycombinator.com")
        .output();
    let dt = t0.elapsed();
    match result {
        Ok(out) if out.status.success() => {
            let html = String::from_utf8_lossy(&out.stdout);
            // Check for JS script tags
            let script_count = html.matches("<script").count();
            println!("[{:.0}ms] ({} bytes, {} <script> tags)", dt.as_millis(), html.len(), script_count);
        }
        _ => println!("FAILED [{:.0}ms]", dt.as_millis()),
    }

    println!("\n=== Benchmark complete ===");
    println!("Note: QuickJS-NG has no JIT — all execution is interpreted.");
    println!("V8 (with JIT) would be 10-100x faster on compute-heavy workloads.");
}
