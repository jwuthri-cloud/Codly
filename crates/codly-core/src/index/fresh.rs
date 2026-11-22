//! How far the index is behind the files on disk: when it last read them all, whether it is
//! reading them now, and what a search result says when it is behind.

use std::time::Instant;

use super::pack::clip;

/// The state of one index against its files. Kept under `Index::fresh`.
#[derive(Debug, Default)]
pub(crate) struct Fresh {
    /// Start of the last refresh that worked (unix ms): the index holds the files as they were
    /// then. Stored in the DB too, so it is known after a restart. None before the first one.
    pub refreshed_at: Option<i64>,
    /// End of the last refresh, whether it worked or not: the next one is due some time later.
    pub checked: Option<Instant>,
    /// Refreshes in progress (one reads the files, any other waits for the writer).
    pub running: usize,
    /// When the first of the refreshes in progress started.
    pub running_since: Option<Instant>,
    /// Why the last refresh failed. None when it worked.
    pub error: Option<String>,
}

impl Fresh {
    /// What a search result says when the index may not match the files; None when it does.
    pub fn warning(&self, now_ms: i64) -> Option<String> {
        let state = if self.running > 0 {
            "is being updated".to_string()
        } else if let Some(e) = &self.error {
            format!("could not be updated ({})", clip(e, 200))
        } else if self.checked.is_none() {
            // Opened, and not compared with the files since: what it holds is from an earlier run.
            "has not been updated yet".to_string()
        } else {
            return None;
        };
        let last = match self.refreshed_at {
            Some(at) => format!("it last read all the files {} ago", age(now_ms - at)),
            None => "it has not read all the files yet".to_string(),
        };
        Some(format!("The code index {state}: {last}. Results may miss files or changes: check with rg."))
    }
}

/// "40 s", "5 min", "3 h", "2 days": a length of time for a warning.
fn age(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..=89 => format!("{s} s"),
        90..=5399 => format!("{} min", (s + 30) / 60),
        5400..=172_799 => format!("{} h", (s + 1800) / 3600),
        _ => format!("{} days", (s + 43_200) / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_that_matches_the_files_warns_about_nothing() {
        let f = Fresh { refreshed_at: Some(1_000), checked: Some(Instant::now()), ..Fresh::default() };
        assert_eq!(f.warning(5_000), None);
    }

    #[test]
    fn the_warning_says_what_is_wrong_and_how_old_the_index_is() {
        let new = Fresh::default();
        let w = new.warning(0).unwrap();
        assert!(w.contains("has not been updated yet") && w.contains("has not read all the files yet"), "{w}");

        // Opened again after a restart: what it holds is two days old until the first refresh.
        let reopened = Fresh { refreshed_at: Some(0), ..Fresh::default() };
        let w = reopened.warning(2 * 86_400_000).unwrap();
        assert!(w.contains("has not been updated yet") && w.contains("2 days ago"), "{w}");

        let first_build = Fresh { running: 1, ..Fresh::default() };
        let w = first_build.warning(0).unwrap();
        assert!(w.contains("is being updated") && w.contains("has not read all the files yet"), "{w}");

        let updating = Fresh { refreshed_at: Some(0), checked: Some(Instant::now()), running: 1, ..Fresh::default() };
        let w = updating.warning(40_000).unwrap();
        assert!(w.contains("is being updated") && w.contains("40 s ago"), "{w}");

        let failed = Fresh { refreshed_at: Some(0), checked: Some(Instant::now()), error: Some("disk gone".into()), ..Fresh::default() };
        let w = failed.warning(3 * 3_600_000).unwrap();
        assert!(w.contains("could not be updated (disk gone)") && w.contains("3 h ago"), "{w}");
    }

    #[test]
    fn ages_read_in_the_unit_that_fits() {
        assert_eq!(age(-5), "0 s");
        assert_eq!(age(89_000), "89 s");
        assert_eq!(age(90_000), "2 min");
        assert_eq!(age(45 * 60_000), "45 min");
        assert_eq!(age(26 * 3_600_000), "26 h");
        assert_eq!(age(3 * 86_400_000), "3 days");
    }
}
