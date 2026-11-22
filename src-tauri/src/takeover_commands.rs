//! "Open in my terminal" (agent sheet): thin wrappers over `Core`'s take-over calls, plus the
//! terminal itself. The core decides who may go and builds the command line; this file only
//! opens a terminal with that line typed, and closes it when the agent is given back. The UI
//! sends an agent id, never a command.

use codly_core::orchestrator::takeover::{self, TakeoverState};
use tauri::State;

use crate::commands::AppState;

type R<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub async fn agent_takeover(state: State<'_, AppState>, agent_id: String) -> R<TakeoverState> {
    state.core().await?.agent_takeover(&agent_id).await.map_err(err)
}

#[tauri::command]
pub async fn take_over_agent(state: State<'_, AppState>, agent_id: String) -> R<TakeoverState> {
    state.core().await?.take_over_agent(&agent_id).await.map_err(err)
}

/// A terminal (attach with `pty_attach`) in the worktree of an agent that is in your terminal,
/// with the CLI's resume command typed and not run. A terminal opened for it before is closed.
#[tauri::command]
pub async fn open_takeover_terminal(state: State<'_, AppState>, agent_id: String) -> R<String> {
    let core = state.core().await?;
    let Some(handed) = core.agent_takeover(&agent_id).await.map_err(err)?.takeover else {
        return Err("this agent is not in your terminal: open it in your terminal first".into());
    };
    let mut env = state.shell_env().await.clone();
    codly_core::env::refresh_path(&mut env); // a CLI installed since the env was captured is found
    takeover::open_terminal(&state.pty, &env, &handed).map_err(err)
}

/// The terminal Codly opened for the agent goes first: a CLI left open in it would write to the
/// session that Codly resumes next.
#[tauri::command]
pub async fn give_back_agent(state: State<'_, AppState>, agent_id: String) -> R<TakeoverState> {
    let core = state.core().await?;
    takeover::close_terminal(&state.pty, &agent_id);
    core.give_back_agent(&agent_id).await.map_err(err)
}
