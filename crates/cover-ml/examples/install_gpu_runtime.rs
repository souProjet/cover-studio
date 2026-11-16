//! `cargo run -p cover-ml --release --example install_gpu_runtime`

use std::sync::atomic::AtomicBool;

fn main() {
    let status = cover_ml::gpu_runtime::status();
    println!("{status:?}");
    let mut last = 0u64;
    let r = cover_ml::gpu_runtime::install(&AtomicBool::new(false), &mut |done, total, pkg| {
        let pct = done * 100 / total.max(1);
        if pct != last {
            last = pct;
            println!("{pct:>3}% {pkg}");
        }
    });
    println!("{r:?}");
}
