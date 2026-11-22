//! The log file and the panic hook: what a user can attach to a bug report. An app started from
//! Finder has no stdout, so every line also goes to `<data dir>/logs/codly.log`. That file stays
//! under 5 MB; the one before it is kept as `codly.1.log`.

use std::any::Any;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tracing::Level;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::prelude::*;

/// The folder of the log files, inside the app's data dir.
pub const DIR: &str = "logs";
pub const FILE: &str = "codly.log";
const OLD_FILE: &str = "codly.1.log";
const MAX_BYTES: u64 = 5 << 20;
/// Lines logged before the data dir is known wait in memory, up to this size.
const MAX_WAITING: usize = 64 << 10;
/// How far back `last_lines` reads in each file.
const TAIL_BYTES: u64 = 256 << 10;

enum Sink {
    /// The data dir comes from the built Tauri app: until `open`, lines wait here.
    Waiting(Vec<u8>),
    Open(LogFile),
}

static SINK: Mutex<Sink> = Mutex::new(Sink::Waiting(Vec::new()));

fn sink() -> MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(PoisonError::into_inner)
}

struct LogFile {
    dir: PathBuf,
    file: File,
    len: u64,
    max: u64,
}

impl LogFile {
    fn open(dir: &Path, max: u64) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        // Only you can read it: lines may name your folders.
        let file = OpenOptions::new().create(true).append(true).mode(0o600).open(dir.join(FILE))?;
        let len = file.metadata()?.len();
        Ok(Self { dir: dir.to_path_buf(), file, len, max })
    }

    /// Appends `lines`. A file that would pass the cap becomes `codly.1.log` first.
    fn write(&mut self, lines: &[u8]) -> io::Result<()> {
        if self.len > 0 && self.len + lines.len() as u64 > self.max {
            std::fs::rename(self.dir.join(FILE), self.dir.join(OLD_FILE))?;
            *self = Self::open(&self.dir, self.max)?;
        }
        self.file.write_all(lines)?;
        self.len += lines.len() as u64;
        Ok(())
    }
}

/// What the log layer writes to: the file, or the waiting room before `open`.
struct SinkWriter;

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut *sink() {
            Sink::Waiting(lines) if lines.len() + buf.len() <= MAX_WAITING => lines.extend_from_slice(buf),
            // The file never opened (`run` said so once): later lines are on stdout only.
            Sink::Waiting(_) => {}
            Sink::Open(file) => file.write(buf)?,
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// What the log file takes of the lines `CODLY_LOG` lets through: all of Codly's own, and of
/// a library (hyper, reqwest, wry…) only `info` and above. Nobody checked what their debug
/// lines say, and the file ends up in bug reports.
fn file_takes(level: &Level, target: &str) -> bool {
    *level <= Level::INFO || target.starts_with("codly")
}

/// Starts logging to stdout and to the log file, and makes every panic log itself. Call it
/// first. `CODLY_LOG` sets the level (default `info`).
pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("CODLY_LOG").unwrap_or_else(|_| "info".into());
    let file = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(|| SinkWriter).with_filter(filter_fn(|meta| file_takes(meta.level(), meta.target())));
    tracing_subscriber::registry().with(filter).with(tracing_subscriber::fmt::layer()).with(file).init();

    // The release build keeps function names (`strip = "debuginfo"`), so the backtrace is readable.
    let print = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line()));
        let line = panic_line(thread.name(), place.as_deref(), info.payload());
        tracing::error!("{line}\n{}", std::backtrace::Backtrace::force_capture());
        print(info);
    }));
}

/// "panic in thread 'main' at src/lib.rs:12: the message".
fn panic_line(thread: Option<&str>, place: Option<&str>, payload: &(dyn Any + Send)) -> String {
    let message = match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
        (Some(s), _) => *s,
        (_, Some(s)) => s.as_str(),
        _ => "no message",
    };
    format!("panic in thread '{}' at {}: {message}", thread.unwrap_or("unnamed"), place.unwrap_or("an unknown place"))
}

/// Opens the log file in `dir` and writes the lines that waited for it.
pub fn open(dir: &Path) -> io::Result<()> {
    let mut file = LogFile::open(dir, MAX_BYTES)?;
    let mut sink = sink();
    if let Sink::Waiting(lines) = &*sink {
        file.write(lines)?;
    }
    *sink = Sink::Open(file);
    Ok(())
}

/// The folder of the open log file; None when it could not be opened.
pub fn dir() -> Option<PathBuf> {
    match &*sink() {
        Sink::Open(file) => Some(file.dir.clone()),
        Sink::Waiting(_) => None,
    }
}

/// The last `n` lines logged in `dir`, oldest first, across the last rotation.
pub fn last_lines(dir: &Path, n: usize) -> io::Result<Vec<String>> {
    let mut text = match read_end(&dir.join(OLD_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    text.push_str(&read_end(&dir.join(FILE))?);
    let lines: Vec<&str> = text.lines().collect();
    Ok(lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect())
}

/// The last `TAIL_BYTES` of a file (its first line may be cut).
fn read_end(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_file_stays_under_its_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut log = LogFile::open(tmp.path(), 100).unwrap();
        let line = |c: char| format!("{}\n", c.to_string().repeat(59)).into_bytes();
        log.write(&line('a')).unwrap();
        assert!(!tmp.path().join(OLD_FILE).exists(), "one line fits");
        log.write(&line('b')).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(OLD_FILE)).unwrap(), line('a'));
        assert_eq!(std::fs::read(tmp.path().join(FILE)).unwrap(), line('b'));
        // Only one old file is kept.
        log.write(&line('c')).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(OLD_FILE)).unwrap(), line('b'));
        assert_eq!(std::fs::read(tmp.path().join(FILE)).unwrap(), line('c'));

        // A new start appends to the file it finds, and rotates it when it is full.
        let mut log = LogFile::open(tmp.path(), 100).unwrap();
        assert_eq!(log.len, 60);
        log.write(&line('d')).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(OLD_FILE)).unwrap(), line('c'));
        // A line longer than the cap is still written, alone in its file.
        log.write(&[b'e'; 300]).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(FILE)).unwrap().len(), 300);
    }

    #[test]
    fn last_lines_read_across_the_rotation() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(last_lines(tmp.path(), 5).is_err(), "no log file yet");
        std::fs::write(tmp.path().join(FILE), "three\nfour\n").unwrap();
        assert_eq!(last_lines(tmp.path(), 5).unwrap(), ["three", "four"]);
        std::fs::write(tmp.path().join(OLD_FILE), "one\ntwo\n").unwrap();
        assert_eq!(last_lines(tmp.path(), 3).unwrap(), ["two", "three", "four"]);
        assert_eq!(last_lines(tmp.path(), 9).unwrap(), ["one", "two", "three", "four"]);
    }

    #[test]
    fn the_log_file_takes_debug_lines_from_codly_only() {
        for target in ["codly_core::index::query", "codly_lib::web", "hyper::proto::h1::io", "reqwest::connect"] {
            for level in [Level::ERROR, Level::WARN, Level::INFO] {
                assert!(file_takes(&level, target), "{level} {target}");
            }
        }
        for level in [Level::DEBUG, Level::TRACE] {
            assert!(file_takes(&level, "codly_core::index::query") && file_takes(&level, "codly_lib::web"), "{level}");
            assert!(!file_takes(&level, "hyper::proto::h1::io") && !file_takes(&level, "reqwest::connect"), "{level}");
        }
    }

    #[test]
    fn a_panic_line_says_where_and_what() {
        assert_eq!(panic_line(Some("main"), Some("src/lib.rs:12"), &"boom"), "panic in thread 'main' at src/lib.rs:12: boom");
        assert_eq!(panic_line(None, None, &String::from("index out of bounds")), "panic in thread 'unnamed' at an unknown place: index out of bounds");
        assert_eq!(panic_line(Some("worker"), None, &42), "panic in thread 'worker' at an unknown place: no message");
    }
}
