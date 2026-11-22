// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use codly_lib::cli::{self, Cli};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match cli::parse(&args) {
        // `codly --help`, `codly --version`: answer and exit, never open a window.
        Ok(Cli::Print(text)) => println!("{text}"),
        // `codly mcp`: agent CLIs launch this binary as their stdio MCP server; it proxies tool
        // calls to the running app over a Unix socket. Never opens a window.
        Ok(Cli::Mcp) => {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
            if let Err(e) = rt.block_on(codly_core::mcp::bridge::run_stdio()) {
                eprintln!("codly mcp: {e:#}");
                std::process::exit(1);
            }
        }
        Ok(Cli::App) => codly_lib::run(),
        // A typo (`codly server`) must not open the app.
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
