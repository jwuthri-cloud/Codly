//! Keep the Mac awake while agents work (`Settings::keep_awake`).
//!
//! A Mac that is left alone goes to sleep after a few minutes, and a sleeping Mac stops every
//! agent in the middle of its turn: a plan that should run for an hour is cut off when you walk
//! away. While work is under way (`Core::activity`), Codly holds `/usr/bin/caffeinate -i`, the
//! system's own tool for this: no idle sleep, and the display may still turn off. The tool is
//! started with `-w <Codly's process id>`, so it ends by itself when Codly does, also after a
//! crash; Codly ends it as soon as nothing runs. Closing the lid still puts the Mac to sleep.

use std::io;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use codly_core::Core;

/// How often Codly looks whether work is under way.
const CHECK_EVERY: Duration = Duration::from_secs(10);
/// How long the Mac is kept awake after work was last seen. Between two steps of a plan (a
/// task is merged and the next starts, a worker that failed waits to start again) nothing runs
/// for a moment, and a Mac that is let go at that moment can go to sleep at once.
const HOLD: Duration = Duration::from_secs(120);

/// Should the Mac be kept awake at `now`? `on` is the Settings switch, `busy` whether work is
/// under way right now, and `last_busy` when it last was (kept up to date here).
fn wanted(on: bool, busy: bool, now: Instant, last_busy: &mut Option<Instant>) -> bool {
    if busy {
        *last_busy = Some(now);
    }
    on && last_busy.is_some_and(|seen| now.duration_since(seen) < HOLD)
}

/// The helper process that keeps the Mac awake while it runs.
struct Awake {
    start: fn() -> io::Result<Child>,
    helper: Option<Child>,
    /// A start that failed was logged: it is not logged at every check.
    failed: bool,
}

impl Awake {
    fn new(start: fn() -> io::Result<Child>) -> Awake {
        Awake { start, helper: None, failed: false }
    }

    /// Start the helper, or end it. A helper that ended by itself (someone killed it) is
    /// started again while it is wanted.
    fn set(&mut self, on: bool) {
        if let Some(helper) = &mut self.helper
            && !matches!(helper.try_wait(), Ok(None))
        {
            self.helper = None;
        }
        match (on, self.helper.take()) {
            (true, None) => match (self.start)() {
                Ok(helper) => {
                    self.helper = Some(helper);
                    self.failed = false;
                }
                Err(e) if !self.failed => {
                    tracing::warn!("cannot keep the Mac awake: {e}");
                    self.failed = true;
                }
                Err(_) => {}
            },
            (true, running) => self.helper = running,
            (false, Some(mut helper)) => {
                let _ = helper.kill();
                let _ = helper.wait();
            }
            (false, None) => {}
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.set(false);
    }
}

/// `caffeinate -i -w <pid>`: no idle sleep until this process is ended, or Codly is gone.
fn caffeinate() -> io::Result<Child> {
    Command::new("/usr/bin/caffeinate")
        .args(caffeinate_args(std::process::id()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// `-i`: no idle sleep (the display may still sleep). `-w <pid>`: until that process ends.
fn caffeinate_args(pid: u32) -> [String; 3] {
    ["-i".into(), "-w".into(), pid.to_string()]
}

/// Keep the Mac awake while work is under way, for as long as Codly runs. Only macOS has the
/// tool; elsewhere (`codly serve` on a server) there is nothing to do.
pub async fn run(core: Arc<Core>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let mut awake = Awake::new(caffeinate);
    let mut last_busy = None;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tick.tick().await;
        match core.activity() {
            Ok(activity) => awake.set(wanted(core.settings().keep_awake, activity.busy(), Instant::now(), &mut last_busy)),
            // Left as it is: the next check decides.
            Err(e) => tracing::warn!("cannot tell whether agents work: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands for `caffeinate`: a process that runs until it is ended.
    fn stand_in() -> io::Result<Child> {
        Command::new("sleep").arg("30").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    }

    fn missing() -> io::Result<Child> {
        Command::new("/nonexistent/caffeinate").spawn()
    }

    fn pid(awake: &Awake) -> Option<u32> {
        awake.helper.as_ref().map(Child::id)
    }

    #[test]
    fn caffeinate_holds_off_idle_sleep_only_while_codly_runs() {
        assert_eq!(caffeinate_args(4242), ["-i", "-w", "4242"]);
    }

    #[test]
    fn awake_while_work_is_under_way_and_for_a_moment_after() {
        let start = Instant::now();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let mut last = None;
        assert!(!wanted(true, false, at(0), &mut last), "nothing has run yet");
        assert!(wanted(true, true, at(10), &mut last));
        // Between two tasks nothing runs for a moment: still awake.
        assert!(wanted(true, false, at(20), &mut last));
        assert!(wanted(true, false, at(10) + HOLD - Duration::from_secs(1), &mut last));
        assert!(!wanted(true, false, at(10) + HOLD, &mut last), "long enough without work: the Mac may sleep");
        // The switch is off: never, also in the middle of work. On again: at once.
        assert!(!wanted(false, true, at(500), &mut last));
        assert!(wanted(true, false, at(510), &mut last));
    }

    #[test]
    fn the_helper_runs_only_while_it_is_wanted() {
        let mut awake = Awake::new(stand_in);
        awake.set(false);
        assert_eq!(pid(&awake), None);
        awake.set(true);
        let first = pid(&awake).expect("the helper runs");
        awake.set(true);
        assert_eq!(pid(&awake), Some(first), "one helper, not one per check");

        // Someone ended it: the next check starts it again.
        let helper = awake.helper.as_mut().unwrap();
        helper.kill().unwrap();
        helper.wait().unwrap();
        awake.set(true);
        let second = pid(&awake).expect("the helper runs again");
        assert_ne!(second, first);

        // Nothing runs any more: the helper is ended.
        awake.set(false);
        assert_eq!(pid(&awake), None);
    }

    #[test]
    fn a_helper_that_cannot_start_is_tried_again_later() {
        let mut awake = Awake::new(missing);
        awake.set(true);
        assert!(pid(&awake).is_none() && awake.failed);
        awake.start = stand_in;
        awake.set(true);
        assert!(pid(&awake).is_some() && !awake.failed, "it starts once it can");
    }
}
