use rquickjs::{Context, Runtime};

fn main() {
    let rt = Runtime::new().expect("create runtime");
    let ctx = Context::full(&rt).expect("create context");

    ctx.with(|ctx| {
        // 1. trivial arithmetic
        let a: i64 = ctx.eval("1 + 1").expect("eval 1+1");
        println!("[spike] 1 + 1 = {a}");

        // 2. ES2020 feature probes (optional chaining, nullish, BigInt)
        let b: i64 = ctx
            .eval("const o = {x:{y:41}}; (o?.x?.y ?? 0) + 1")
            .expect("eval optional chaining");
        println!("[spike] optional-chaining/?? = {b}");

        let big: String = ctx.eval("(2n ** 64n).toString()").expect("eval BigInt");
        println!("[spike] 2n ** 64n = {big}");

        let ty: String = ctx
            .eval("typeof globalThis")
            .expect("eval typeof globalThis");
        println!("[spike] typeof globalThis = {ty}");
    });

    println!("[spike] OK");
}
