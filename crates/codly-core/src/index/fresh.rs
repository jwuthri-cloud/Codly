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
    /// When the first search stops waiting for the refreshes in progress. The comparison a
    /// search asks for ends long before that, and no result has to mention it. One that is
    /// still in progress then reads many files (a branch switch, a big pull): from then on
    /// results say that the index is being updated. None when no search asked for one.
    pub late_from: Option<Instant>,
    /// Why the last refresh failed. None when it worked.
    pub error: Option<String>,
}

impl Fresh {
    /// What a search result says when the index may not match the files; None when it does.
    pub fn warning(&self, now_ms: i64) -> Option<String> {
        // A refresh in progress is worth a word when the index was not compared with the files
        // since it was opened (a first build), or when a search gave up waiting for it. Not
        // the short comparison another search started a moment ago: the index matched the
        // files a few seconds ago, which is the state that gives no warning.
        let behind = self.running > 0 && (self.checked.is_none() || self.late_from.is_some_and(|at| at <= Instant::now()));
        let state = if behind {
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
    use std::time::Duration;

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

        // A search stopped waiting for this refresh: it takes long.
        let updating =
            Fresh { refreshed_at: Some(0), checked: Some(Instant::now()), running: 1, late_from: Some(Instant::now()), ..Fresh::default() };
        let w = updating.warning(40_000).unwrap();
        assert!(w.contains("is being updated") && w.contains("40 s ago"), "{w}");

        let failed = Fresh { refreshed_at: Some(0), checked: Some(Instant::now()), error: Some("disk gone".into()), ..Fresh::default() };
        let w = failed.warning(3 * 3_600_000).unwrap();
        assert!(w.contains("could not be updated (disk gone)") && w.contains("3 h ago"), "{w}");
    }

    #[test]
    fn the_short_check_of_another_search_is_not_worth_a_warning() {
        let checked = Fresh { refreshed_at: Some(1_000), checked: Some(Instant::now()), ..Fresh::default() };
        // A search claimed the next comparison just now, and stops waiting for it in a minute.
        let waited_for = Fresh { running: 1, late_from: Some(Instant::now() + Duration::from_secs(60)), ..checked };
        assert_eq!(waited_for.warning(5_000), None);
        // Nobody waits for this one (it follows a merge).
        let unasked = Fresh { late_from: None, ..waited_for };
        assert_eq!(unasked.warning(5_000), None);

        // What the last refresh that ended found still counts while the next one runs.
        let failed = Fresh { error: Some("disk gone".into()), ..unasked };
        assert!(failed.warning(5_000).unwrap().contains("could not be updated (disk gone)"));
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
