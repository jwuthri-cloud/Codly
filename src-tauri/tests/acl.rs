//! Every command in `generate_handler!` must be in build.rs's app manifest and allowed by the
//! main capability; otherwise the desktop app rejects it ("not allowed by ACL") while the browser
//! demo, which mocks the core, keeps working.

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(String::from).collect()
}

#[test]
fn every_command_is_allowed() {
    let lib = include_str!("../src/lib.rs");
    let start = lib.find("generate_handler![").expect("generate_handler! in lib.rs");
    let body = &lib[start..start + lib[start..].find(']').unwrap()];
    let handlers: Vec<String> = body.lines().skip(1).flat_map(|l| words(l.split("::").last().unwrap_or(""))).collect();
    let manifest = include_str!("../build.rs");
    let capability = include_str!("../capabilities/main.json");
    for h in &handlers {
        assert!(manifest.contains(&format!("\"{h}\"")), "{h} is missing from build.rs's AppManifest");
        assert!(capability.contains(&format!("\"allow-{}\"", h.replace('_', "-"))), "{h} is missing from capabilities/main.json");
    }
    assert!(handlers.len() > 50, "parsed only {} handlers", handlers.len());
}
