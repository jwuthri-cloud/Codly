// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `codly mcp`: agent CLIs launch this binary as their stdio MCP server; it proxies tool
    // calls to the running app over a Unix socket. Never opens a window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
        if let Err(e) = rt.block_on(codly_core::mcp::bridge::run_stdio()) {
            eprintln!("codly mcp: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    codly_lib::run();
}
