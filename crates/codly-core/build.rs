//! Builds in the ready-made lots: each known design in `src/office/lots/` that exists (a design
//! added later, like the Small office, is picked up once its file lands).

use std::path::Path;

const KNOWN: &[&str] = &["garage", "small-office", "studio"];

fn main() {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../src/office/lots");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut out = String::from("/// (name, JSON) of every ready-made lot.\npub const TEMPLATES: &[(&str, &str)] = &[\n");
    for name in KNOWN {
        let file = dir.join(format!("{name}.json"));
        if let Ok(file) = file.canonicalize() {
            out += &format!("    ({name:?}, include_str!({:?})),\n", file.display().to_string());
        }
    }
    out += "];\n";
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("templates.rs"), out).unwrap();
}
