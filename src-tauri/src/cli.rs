//! What `codly <arguments>` asks for, decided before a window or a server starts.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Codly runs coding CLIs (Claude Code, Codex, Gemini CLI, OpenCode) as one team.

Usage:
  codly                 Open the app.
  codly serve [--port N] [--host ADDR] [--allow-remote]
                        Run without a window and serve the app to a browser. It prints a
                        link with a token. The default is this Mac only (127.0.0.1:4280).
                        A --host that other machines can reach also needs --allow-remote:
                        the link then works for anyone on your network, over plain HTTP.
                        An SSH tunnel is safer: ssh -L 4280:127.0.0.1:4280 <this Mac>
  codly mcp             The MCP server that agents talk to. Codly starts it for them.
  codly --version       Print the version.
  codly --help          Print this help.

Environment:
  CODLY_LOG=debug       Log more. Settings > General > Troubleshooting shows the log file.";

#[derive(Debug, PartialEq)]
pub enum Cli {
    /// Open the window, or `serve` (`web::from_args` checks its options).
    App,
    /// `codly mcp`: the stdio MCP bridge.
    Mcp,
    /// Print this and exit: the help or the version.
    Print(String),
}

/// `args` includes the program. Err = a command Codly does not know, with the help.
pub fn parse(args: &[String]) -> Result<Cli, String> {
    let Some(first) = args.get(1) else { return Ok(Cli::App) };
    let wants_help = |a: &String| a == "--help" || a == "-h";
    match first.as_str() {
        "--help" | "-h" | "help" => Ok(Cli::Print(HELP.into())),
        "--version" | "-V" => Ok(Cli::Print(format!("codly {VERSION}"))),
        "mcp" => Ok(Cli::Mcp),
        "serve" if args[2..].iter().any(wants_help) => Ok(Cli::Print(HELP.into())),
        "serve" => Ok(Cli::App),
        // macOS and `tauri dev` may pass flags of their own: those still open the app.
        flag if flag.starts_with('-') => Ok(Cli::App),
        word => Err(format!("codly: unknown command “{word}”\n\n{HELP}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(line: &str) -> Result<Cli, String> {
        parse(&line.split(' ').map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn help_and_version_print_and_exit() {
        assert_eq!(cli("codly --version"), Ok(Cli::Print(format!("codly {VERSION}"))));
        assert_eq!(cli("codly -V"), cli("codly --version"));
        for line in ["codly --help", "codly -h", "codly help", "codly serve --help", "codly serve --port 9000 -h"] {
            let Ok(Cli::Print(help)) = cli(line) else { panic!("{line} should print the help") };
            assert!(help.contains("codly serve [--port N] [--host ADDR] [--allow-remote]") && help.contains("codly mcp"), "{help}");
        }
    }

    #[test]
    fn the_app_serve_and_mcp_still_start() {
        assert_eq!(cli("codly"), Ok(Cli::App));
        assert_eq!(cli("codly serve --port 9000"), Ok(Cli::App));
        assert_eq!(cli("codly mcp"), Ok(Cli::Mcp));
        assert_eq!(cli("codly -psn_0_12345"), Ok(Cli::App), "a flag from the system opens the app");
    }

    #[test]
    fn a_typo_does_not_open_the_window() {
        let e = cli("codly server").unwrap_err();
        assert!(e.starts_with("codly: unknown command “server”") && e.contains("Usage:"), "{e}");
    }

    #[test]
    fn the_version_has_one_source() {
        // The source is [workspace.package] in the root Cargo.toml (docs/RELEASE.md). A second
        // number in one of these files could disagree with what the app and its report show.
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(conf.get("version").is_none(), "tauri.conf.json must not set a version: without one, the app bundle takes this crate's");
        let package: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        assert!(package.get("version").is_none(), "package.json must not set a version: nothing reads it");
        assert!(include_str!("../Cargo.toml").contains("\nversion.workspace = true\n"), "src-tauri/Cargo.toml must take the workspace version");
        assert_eq!(VERSION, codly_core::version(), "the app and the engine have the same version");
    }
}
