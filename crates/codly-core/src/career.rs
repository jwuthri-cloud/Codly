//! Careers: XP, ranks, skills and traits, from real events only. Pure functions, no I/O.
//!
//! An agent starts as an Intern and is promoted by its track record (commits, passing checks,
//! approved reviews, merges). What makes a senior agent stronger is real: its own lessons
//! and memory ride on its prompts, the traits you chose shape how it works, and a promotion
//! is the moment the app offers you a stronger model (you decide; nothing changes by itself).

use crate::model::{Applicant, Career, Milestone, Rank};

/// XP to reach each rank above Intern.
pub const RANKS: [(Rank, i64); 3] = [(Rank::Junior, 60), (Rank::Senior, 250), (Rank::Staff, 800)];
const MAX_MILESTONES: usize = 50;
const MAX_SKILLS: usize = 24;

/// (id, label, how it changes the work: None = personality only, shown in the office).
pub const TRAITS: [(&str, &str, Option<&str>); 8] = [
    ("perfectionist", "Perfectionist", Some("Before you say you are done, re-read your whole diff and run the checks again.")),
    ("bookworm", "Bookworm", Some("Before editing, read the code and docs around what you change.")),
    ("lazy", "Lazy", Some("Prefer the smallest change that works; do not add what was not asked.")),
    ("social", "Social", Some("When something is unclear, ask (the Lead or the user) instead of guessing.")),
    ("clumsy", "Clumsy", None),
    ("hotheaded", "Hot-headed", None),
    ("competitive", "Competitive", None),
    ("nightowl", "Night owl", None),
];
/// Pairs that make no sense together (same list as the office's traits.ts).
const CLASH: [(&str, &str); 3] = [("lazy", "perfectionist"), ("lazy", "competitive"), ("bookworm", "social")];

pub fn rank_for(xp: i64) -> Rank {
    RANKS.iter().rev().find(|(_, need)| xp >= *need).map_or(Rank::Intern, |(r, _)| *r)
}

pub fn rank_label(r: Rank) -> &'static str {
    match r {
        Rank::Intern => "Intern",
        Rank::Junior => "Junior",
        Rank::Senior => "Senior",
        Rank::Staff => "Staff",
    }
}

/// The area a changed file trains.
pub fn skill_for(path: &str) -> Option<&'static str> {
    let p = path.to_ascii_lowercase();
    let ext = p.rsplit_once('.').map_or("", |(_, e)| e);
    let name = p.rsplit('/').next().unwrap_or(&p);
    if name.contains(".test.") || name.contains(".spec.") || name.ends_with("_test.rs") || name.ends_with("_test.go") || name.ends_with("_test.py") || p.starts_with("tests/") || p.starts_with("test/") || p.contains("/tests/") || p.contains("/test/") {
        return Some("Testing");
    }
    if ext == "sql" || ext == "prisma" || p.contains("migration") {
        return Some("Databases");
    }
    Some(match ext {
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "svelte" | "vue" => "UI",
        "css" | "scss" | "less" => "Styling",
        "rs" => "Rust",
        "py" => "Python",
        "go" => "Go",
        "md" | "mdx" | "txt" => "Writing",
        _ => return None,
    })
}

/// Something it really did.
pub enum Credit<'a> {
    Commit,
    Check { ok: bool },
    /// Its work was reviewed.
    Review { approved: bool },
    /// It reviewed someone else's work.
    Reviewed,
    Merge,
    Files(&'a [String]),
}

/// Apply a credit. Returns the new rank when it was just promoted.
pub fn credit(c: &mut Career, what: Credit, now: i64) -> Option<Rank> {
    let before = c.rank;
    let mut gain = 0;
    match what {
        Credit::Commit => {
            c.stats.commits += 1;
            gain = 15;
            if c.stats.commits == 1 {
                note(c, now, "First commit".into());
            }
        }
        Credit::Check { ok: true } => {
            c.stats.checks_passed += 1;
            gain = 10;
        }
        Credit::Check { ok: false } => c.stats.checks_failed += 1,
        Credit::Review { approved: true } => {
            c.stats.approved += 1;
            gain = 20;
        }
        Credit::Review { approved: false } => c.stats.changes_requested += 1,
        Credit::Reviewed => {
            c.stats.reviews_done += 1;
            gain = 10;
        }
        Credit::Merge => {
            c.stats.merged += 1;
            gain = 30;
            let n = c.stats.merged;
            if n == 1 || n % 10 == 0 {
                note(c, now, if n == 1 { "First merge".into() } else { format!("{n} merges") });
            }
        }
        Credit::Files(paths) => {
            for s in paths.iter().filter_map(|p| skill_for(p)) {
                if c.skills.len() < MAX_SKILLS || c.skills.contains_key(s) {
                    *c.skills.entry(s.to_string()).or_default() += 1;
                }
            }
        }
    }
    c.xp += gain;
    c.rank = c.rank.max(rank_for(c.xp));
    (c.rank > before).then(|| {
        note(c, now, format!("Promoted to {}", rank_label(c.rank)));
        c.rank
    })
}

pub fn note(c: &mut Career, at: i64, text: String) {
    c.milestones.push(Milestone { at, text });
    let over = c.milestones.len().saturating_sub(MAX_MILESTONES);
    c.milestones.drain(..over);
}

/// What its traits and track record add in front of its prompts (empty: nothing to add).
pub fn briefing(c: &Career) -> String {
    let mut out = String::new();
    for (_, label, rule) in TRAITS.iter().filter(|(id, ..)| c.traits.iter().any(|t| t == id)) {
        if let Some(rule) = rule {
            out.push_str(&format!("{label}: {rule}\n"));
        }
    }
    if c.xp > 0 {
        let mut top: Vec<_> = c.skills.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let areas = top.iter().take(3).map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ");
        out.push_str(&format!(
            "Your track record: {} ({} XP), {} merged, {} checks passed, {} approved reviews{}.\n",
            rank_label(c.rank),
            c.xp,
            c.stats.merged,
            c.stats.checks_passed,
            c.stats.approved,
            if areas.is_empty() { String::new() } else { format!("; strongest in {areas}") }
        ));
    }
    out
}

/// Keep only known traits, at most 3, no duplicates.
pub fn clean_traits(ts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for t in ts {
        if TRAITS.iter().any(|(id, ..)| id == t) && !out.contains(t) && out.len() < 3 {
            out.push(t.clone());
        }
    }
    out
}

/// A career that came from a file (an imported agent): bounded and consistent.
pub fn sanitize(mut c: Career) -> Career {
    c.xp = c.xp.clamp(0, 1_000_000);
    c.rank = rank_for(c.xp);
    c.traits = clean_traits(&c.traits);
    c.skills.retain(|k, v| skill_names().contains(&k.as_str()) && *v >= 0);
    c.milestones.retain(|m| !m.text.is_empty() && m.text.len() <= 120 && !m.text.chars().any(char::is_control));
    let over = c.milestones.len().saturating_sub(MAX_MILESTONES);
    c.milestones.drain(..over);
    for v in [&mut c.stats.commits, &mut c.stats.checks_passed, &mut c.stats.checks_failed, &mut c.stats.approved, &mut c.stats.changes_requested, &mut c.stats.reviews_done, &mut c.stats.merged] {
        *v = (*v).clamp(0, 1_000_000);
    }
    c
}

fn skill_names() -> [&'static str; 11] {
    ["Testing", "Databases", "TypeScript", "JavaScript", "UI", "Styling", "Rust", "Python", "Go", "Writing", "Reviews"]
}

/// Two traits that go together, from a seed.
pub fn random_traits(seed: u64) -> Vec<String> {
    let n = TRAITS.len() as u64;
    let a = TRAITS[(seed % n) as usize].0;
    for k in 1..n {
        let b = TRAITS[(((seed / n) + k) % n) as usize].0;
        if b != a && !CLASH.iter().any(|&(x, y)| (x == a && y == b) || (x == b && y == a)) {
            return vec![a.into(), b.into()];
        }
    }
    vec![a.into()]
}

/// Build/Buy: (id, label, price in XP). Cosmetic: where each goes is the office's (layout.ts).
pub const CATALOG: [(&str, &str, i64); 8] = [
    ("poster", "Framed print", 40),
    ("ficus", "Ficus", 60),
    ("floor-lamp", "Reading lamp", 80),
    ("cooler", "Water cooler", 100),
    ("armchair", "Armchair", 150),
    ("bookshelf", "Bookshelf", 200),
    ("bench", "Lobby bench", 250),
    ("sofa", "Big sofa", 400),
];

const PITCHES: [&str; 8] = [
    "I read the whole codebase before I touch anything.",
    "Ship small, ship often. That's me.",
    "I'll find the bug you didn't know you had.",
    "Give me the gnarly ticket nobody wants.",
    "Tests first, then everything else.",
    "I ask questions early so we don't redo it later.",
    "Clean diffs, short reviews.",
    "I work best when it's quiet. And late.",
];

/// Applicants for a job: distinct names (none taken), colors and looks, two traits each.
pub fn applicants(n: usize, seed: u64, taken: &[String], colors: &[i64]) -> Vec<Applicant> {
    let mut names: Vec<String> = taken.to_vec();
    let mut used: Vec<i64> = colors.to_vec();
    (0..n)
        .map(|i| {
            let s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(i as u64 * 0x2545_f491) >> 7;
            let name = crate::orchestrator::names::next_name(&names);
            names.push(name.clone());
            let color = crate::orchestrator::names::next_color(&used);
            used.push(color);
            Applicant {
                name,
                color,
                avatar: crate::hire::AVATARS[(s % 8) as usize].to_string(),
                traits: random_traits(s / 8),
                pitch: PITCHES[((seed >> 11) as usize + i * 3) % 8].to_string(), // distinct within an offer
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotions_follow_real_work() {
        let mut c = Career::default();
        assert_eq!(credit(&mut c, Credit::Commit, 1), None);
        assert_eq!(c.milestones[0].text, "First commit");
        assert_eq!(credit(&mut c, Credit::Check { ok: false }, 2), None);
        assert_eq!(c.xp, 15, "a failed check earns nothing");
        assert_eq!(credit(&mut c, Credit::Review { approved: true }, 3), None);
        assert_eq!(credit(&mut c, Credit::Merge, 4), Some(Rank::Junior)); // 15 + 20 + 30 = 65
        assert_eq!(c.milestones.last().unwrap().text, "Promoted to Junior");
        assert_eq!(credit(&mut c, Credit::Commit, 5), None, "no second promotion to the same rank");
    }

    #[test]
    fn skills_and_briefing() {
        let mut c = Career { traits: vec!["perfectionist".into(), "clumsy".into()], ..Default::default() };
        credit(&mut c, Credit::Files(&["src/a.rs".into(), "tests/flow.rs".into(), "README".into()]), 1);
        assert_eq!(c.skills.get("Rust"), Some(&1));
        assert_eq!(c.skills.get("Testing"), Some(&1));
        let b = briefing(&c);
        assert!(b.contains("re-read your whole diff"), "{b}");
        assert!(!b.contains("Clumsy"), "personality-only traits never reach the prompt");
        assert!(!b.contains("track record"), "no XP yet: nothing to brag about");
    }

    #[test]
    fn imported_careers_are_bounded() {
        let mut c = Career { xp: 300, rank: Rank::Staff, traits: vec!["lazy".into(), "rm -rf".into(), "lazy".into()], ..Default::default() };
        c.skills.insert("<script>".into(), 9);
        c.milestones.push(Milestone { at: 1, text: "ok\u{1b}[2J".into() });
        let c = sanitize(c);
        assert_eq!(c.rank, Rank::Senior, "rank follows XP, not the file");
        assert_eq!(c.traits, vec!["lazy".to_string()]);
        assert!(c.skills.is_empty() && c.milestones.is_empty());
    }

    #[test]
    fn applicants_are_distinct() {
        let a = applicants(3, 42, &["Ada".into()], &[0]);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|x| x.name != "Ada" && x.color != 0 && x.traits.len() == 2));
        assert!(a[0].name != a[1].name && a[1].name != a[2].name);
        assert!(a[0].pitch != a[1].pitch && a[1].pitch != a[2].pitch && a[0].pitch != a[2].pitch);
    }
}
