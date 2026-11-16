//! Prints the inputs/outputs of every downloaded model and the provider used.
//!
//! `cargo run -p cover-ml --example inspect`

use cover_ml::runtime::open_session;
use cover_ml::{model_path, Device, MODELS};

fn main() {
    for spec in MODELS {
        let path = model_path(spec);
        if !path.is_file() {
            println!("{}: not downloaded", spec.id);
            continue;
        }
        match open_session(&path, Device::Auto) {
            Ok((session, provider)) => {
                println!("{} on {}", spec.id, provider.label());
                for i in session.inputs() {
                    println!("  in  {} {:?}", i.name(), i.dtype());
                }
                for o in session.outputs() {
                    println!("  out {} {:?}", o.name(), o.dtype());
                }
            }
            Err(e) => println!("{}: {e}", spec.id),
        }
    }
}
