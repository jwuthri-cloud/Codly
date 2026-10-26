//! Interactive terminals for the Workspace's Terminal tab (portable-pty, blocking I/O on
//! one reader thread per terminal). Output is kept in a scrollback ring so a re-attached
//! xterm gets the history, then streamed live to the attached sink.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Receives raw terminal bytes (UTF-8 not guaranteed).
pub type PtySink = Box<dyn Fn(Vec<u8>) + Send + Sync>;

const SCROLLBACK: usize = 512 * 1024;

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    shared: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    scrollback: VecDeque<u8>,
    sink: Option<PtySink>,
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<String, Session>>,
}

impl PtyManager {
    /// Start the user's login shell in `cwd` with the captured login environment.
    pub fn open(&self, cwd: &Path, env: &HashMap<String, String>) -> anyhow::Result<String> {
        let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })?;
        let shell = env.get("SHELL").cloned().unwrap_or_else(|| "/bin/zsh".into());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.arg("-l");
        cmd.cwd(cwd);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave); // so the reader sees EOF when the shell exits
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let id = crate::model::new_id();

        let thread_shared = shared.clone();
        std::thread::Builder::new().name(format!("pty-{id}")).spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = buf[..n].to_vec();
                        let mut s = thread_shared.lock().unwrap();
                        s.scrollback.extend(&chunk);
                        let excess = s.scrollback.len().saturating_sub(SCROLLBACK);
                        s.scrollback.drain(..excess);
                        if let Some(sink) = &s.sink {
                            sink(chunk);
                        }
                    }
                }
            }
        })?;

        self.sessions.lock().unwrap().insert(id.clone(), Session { master: pair.master, writer, child, shared });
        Ok(id)
    }

    /// Replay the scrollback into `sink`, then stream live output to it (replaces any previous sink).
    pub fn attach(&self, id: &str, sink: PtySink) -> anyhow::Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id).ok_or_else(|| anyhow::anyhow!("no terminal {id}"))?;
        let mut s = session.shared.lock().unwrap();
        let history: Vec<u8> = s.scrollback.iter().copied().collect();
        if !history.is_empty() {
            sink(history);
        }
        s.sink = Some(sink);
        Ok(())
    }

    pub fn write(&self, id: &str, data: &[u8]) -> anyhow::Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(id).ok_or_else(|| anyhow::anyhow!("no terminal {id}"))?;
        session.writer.write_all(data)?;
        session.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id).ok_or_else(|| anyhow::anyhow!("no terminal {id}"))?;
        session.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn close(&self, id: &str) {
        if let Some(mut s) = self.sessions.lock().unwrap().remove(id) {
            let _ = s.child.kill();
        }
    }

    pub fn close_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_echo_round_trip() {
        let pty = PtyManager::default();
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.insert("SHELL".into(), "/bin/sh".into());
        let id = pty.open(Path::new("/tmp"), &env).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let tx = Mutex::new(tx);
        pty.attach(&id, Box::new(move |b| { let _ = tx.lock().unwrap().send(b); })).unwrap();
        pty.write(&id, b"echo codly-$((40+2))\n").unwrap();
        let mut seen = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !seen.contains("codly-42") {
            if let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                seen.push_str(&String::from_utf8_lossy(&b));
            }
        }
        pty.close(&id);
        assert!(seen.contains("codly-42"), "got: {seen}");
    }
}
