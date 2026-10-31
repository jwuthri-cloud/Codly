//! Careers: XP, ranks, skills and traits, from real events only. Pure functions, no I/O.
//!
//! An agent starts as an Intern and is promoted by its track record (commits, passing checks,
//! approved reviews, merges). What makes a senior agent stronger is real: its own lessons
//! and memory ride on its prompts, the traits you chose shape how it works, and a promotion
//! is the moment the app offers you a stronger model (you decide; nothing changes by itself).

use crate::model::{Applicant, Career, Milestone, Rank};

/// Extra XP for a review by a mentor: feedback from a senior teaches more.
pub const MENTORED_XP: i64 = 10;
const MAX_FRIENDS: usize = 24;

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

/// What trusting a rank gives an agent, as you grant it (a promotion only offers it).
pub const PERKS: [(Rank, &str); 3] = [
    (Rank::Junior, "Edits files inside its own worktree without asking; commands still ask"),
    (Rank::Senior, "Works inside its own worktree without asking; risky actions still ask"),
    (Rank::Staff, "Its work merges without the cross-vendor review"),
];

/// Whether this rank's perk is on: you granted it and it still holds the rank.
pub fn trusts(c: &Career, r: Rank) -> bool {
    c.trust >= r && c.rank >= r
}

/// Grant trust up to `r` (capped at its rank; Intern takes it all back). Returns the trust set.
pub fn grant(c: &mut Career, r: Rank, now: i64) -> Rank {
    let r = r.min(c.rank);
    if r > c.trust && PERKS.iter().any(|(p, _)| *p == r) {
        note(c, now, format!("Trusted as {}", rank_label(r)));
    }
    c.trust = r;
    r
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

/// Something it really did (except `Cheat`: the developer panel's, which says so).
pub enum Credit<'a> {
    /// XP given from the developer panel, noted as a "Cheat" milestone.
    Cheat(i64),
    Commit,
    Check { ok: bool },
    /// Its work was reviewed.
    Review { approved: bool },
    /// It reviewed someone else's work.
    Reviewed,
    /// A Senior or Staff reviewed its work while it was an Intern or Junior (`mentors`).
    Mentored,
    Merge,
    Files(&'a [String]),
}

/// Apply a credit. Returns the new rank when it was just promoted.
pub fn credit(c: &mut Career, what: Credit, now: i64) -> Option<Rank> {
    let before = c.rank;
    let mut gain = 0;
    match what {
        Credit::Cheat(xp) => {
            gain = xp.max(0);
            note(c, now, format!("Cheat: +{gain} XP"));
        }
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
        Credit::Mentored => gain = MENTORED_XP,
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

/// Whether a review by `reviewer` mentors the author: a Senior or Staff reviewing an Intern or Junior.
pub fn mentors(reviewer: Rank, author: Rank) -> bool {
    reviewer >= Rank::Senior && author <= Rank::Junior
}

/// One more review between it and another agent.
pub fn befriend(c: &mut Career, other: &str) {
    if c.friends.len() < MAX_FRIENDS || c.friends.contains_key(other) {
        *c.friends.entry(other.to_string()).or_default() += 1;
    }
}

pub fn note(c: &mut Career, at: i64, text: String) {
    c.milestones.push(Milestone { at, text });
    let over = c.milestones.len().saturating_sub(MAX_MILESTONES);
    c.milestones.drain(..over);
}

const DAY_MS: f64 = 86_400_000.0;
/// Legendary, the title above Staff: days on the crew, merges and stamina it needs on top of
/// the rank, so it can't be had from XP alone.
pub const LEGEND_DAYS: f64 = 90.0;
pub const LEGEND_MERGES: i64 = 150;
pub const LEGEND_STAMINA: i64 = 80;

/// When it joined the crew: this repo's row, or its record's first milestone when older (an
/// imported agent keeps its days).
pub fn hired_at(c: &Career, created_at: i64) -> i64 {
    c.milestones.iter().map(|m| m.at).filter(|t| *t > 0).fold(created_at, i64::min)
}

/// Stamina (0 … 100) from what it really lived through: days on the crew (up to 35), merges
/// (up to 35) and shifts it worked and rested from (up to 30), each with diminishing returns;
/// minus 2 each time it got sick.
pub fn stamina(c: &Career, days: f64) -> i64 {
    let grow = |x: f64, scale: f64, cap: f64| cap * (1.0 - (-x.max(0.0) / scale).exp());
    let s = grow(days, 45.0, 35.0) + grow(c.stats.merged as f64, 75.0, 35.0) + grow(c.stats.shifts as f64, 40.0, 30.0) - 2.0 * c.stats.sick as f64;
    (s.round() as i64).clamp(0, 100)
}

/// Whether it has earned Legendary: Staff, 90 days on the crew, 150 merges and stamina 80.
pub fn legend_ok(c: &Career, days: f64) -> bool {
    c.rank == Rank::Staff && days >= LEGEND_DAYS && c.stats.merged >= LEGEND_MERGES && stamina(c, days) >= LEGEND_STAMINA
}

/// Bring stamina and the Legend title up to date (the core does it on every save of the
/// agent). Returns true when it just became a Legend (once: the title stays).
pub fn refresh(c: &mut Career, hired_at: i64, now: i64) -> bool {
    let days = (now - hired_at).max(0) as f64 / DAY_MS;
    c.stamina = stamina(c, days);
    if c.legend || !legend_ok(c, days) {
        return false;
    }
    c.legend = true;
    note(c, now, "Legend".into());
    true
}

/// The office reports a shift worked and rested from at home, or a sickness.
pub fn rest(c: &mut Career, sick: bool) {
    if sick {
        c.stats.sick += 1;
    } else {
        c.stats.shifts += 1;
    }
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
    c.trust = Rank::Intern; // trust is earned in each repo, from you
    c.friends.clear(); // ids of agents in the repo it came from
    c.mentor = None;
    c.traits = clean_traits(&c.traits);
    c.skills.retain(|k, v| skill_names().contains(&k.as_str()) && *v >= 0);
    c.milestones.retain(|m| !m.text.is_empty() && m.text.len() <= 120 && !m.text.chars().any(char::is_control));
    let over = c.milestones.len().saturating_sub(MAX_MILESTONES);
    c.milestones.drain(..over);
    let s = &mut c.stats;
    for v in [&mut s.commits, &mut s.checks_passed, &mut s.checks_failed, &mut s.approved, &mut s.changes_requested, &mut s.reviews_done, &mut s.merged, &mut s.shifts, &mut s.sick] {
        *v = (*v).clamp(0, 1_000_000);
    }
    // Recomputed here from its record (`refresh`), never taken from the file.
    c.stamina = 0;
    c.legend = false;
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

/// A piece in the furniture catalog (`src/office/furniture.json`, shared with the frontend).
#[derive(serde::Deserialize)]
struct Piece {
    id: String,
    price: i64,
    /// Earned first: a rank ("senior") someone on the crew must have, or "kitchen".
    #[serde(default)]
    needs: Option<String>,
    /// A counter or a fridge: it makes a kitchen.
    #[serde(default)]
    kitchen: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    builtin: Option<String>,
}

fn catalog(item: &str) -> Option<&'static Piece> {
    static CATALOG: std::sync::OnceLock<Vec<Piece>> = std::sync::OnceLock::new();
    let all = CATALOG.get_or_init(|| serde_json::from_str(include_str!("../../../src/office/furniture.json")).expect("furniture.json is valid"));
    all.iter().find(|e| e.id == item)
}

/// Build/Buy: the price in XP of a piece of furniture (the catalog is the office's, shared with
/// the frontend), or None for no such piece.
pub fn price(item: &str) -> Option<i64> {
    catalog(item).map(|e| e.price)
}

/// One of the office's own kind of pieces (furniture.json `builtin`) that the shop sells: the
/// foosball, the pool table, the PS5. Bought, it stands in the lot like the Studio's own.
pub fn sold_builtin(item: &str) -> bool {
    catalog(item).is_some_and(|e| e.builtin.is_some() && !e.hidden)
}

/// Why a piece can't be bought yet, or None when it can (furniture.json `needs`, mirror of
/// `locked` in src/office/buy.ts): someone on the `crew` at a rank, or a kitchen among the
/// pieces already in the `office` (a counter or a fridge). The shop checks it on every buy.
pub fn locked<'a>(item: &str, crew: &[Rank], office: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let need = catalog(item)?.needs.as_deref()?;
    if need == "kitchen" {
        let kitchen = office.into_iter().any(|i| catalog(i).is_some_and(|e| e.kitchen));
        return (!kitchen).then(|| "Needs a kitchen: a counter or a fridge first".into());
    }
    let Ok(rank) = serde_json::from_value::<Rank>(serde_json::Value::String(need.into())) else { return Some(format!("needs {need}")) };
    let who = match rank {
        Rank::Staff => "on Staff".to_string(),
        Rank::Intern => "an Intern".to_string(),
        r => format!("a {}", rank_label(r)),
    };
    (!crew.iter().any(|r| *r >= rank)).then(|| format!("Someone on the crew must be {who}"))
}

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
    use crate::model::CareerStats;

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
        assert_eq!(c.trust, Rank::Intern, "an import is not trusted here yet");
        assert_eq!(c.traits, vec!["lazy".to_string()]);
        assert!(c.skills.is_empty() && c.milestones.is_empty());
    }

    #[test]
    fn trust_is_granted_never_above_rank() {
        let mut c = Career { xp: 300, rank: Rank::Senior, ..Default::default() };
        assert!(!trusts(&c, Rank::Senior), "a promotion alone grants nothing");
        assert_eq!(grant(&mut c, Rank::Staff, 1), Rank::Senior, "capped at its rank");
        assert!(trusts(&c, Rank::Senior) && !trusts(&c, Rank::Staff));
        assert_eq!(c.milestones.last().unwrap().text, "Trusted as Senior");
        grant(&mut c, Rank::Intern, 2);
        assert!(!trusts(&c, Rank::Senior), "taken back");
        assert_eq!(c.milestones.len(), 1, "taking trust back is not a milestone");
    }

    #[test]
    fn seniors_mentor_juniors() {
        assert!(mentors(Rank::Senior, Rank::Junior) && mentors(Rank::Staff, Rank::Intern));
        assert!(!mentors(Rank::Junior, Rank::Intern) && !mentors(Rank::Staff, Rank::Senior));
        let mut c = Career::default();
        credit(&mut c, Credit::Mentored, 1);
        assert_eq!(c.xp, MENTORED_XP);
        befriend(&mut c, "a1");
        befriend(&mut c, "a1");
        assert_eq!(c.friends.get("a1"), Some(&2));
        c.mentor = Some("a1".into());
        let c = sanitize(c);
        assert!(c.friends.is_empty() && c.mentor.is_none(), "friends are agents of the old repo");
    }

    #[test]
    fn stamina_grows_with_real_history() {
        let mut c = Career::default();
        assert_eq!(stamina(&c, 0.0), 0, "a new hire starts at 0");
        let a = stamina(&c, 30.0);
        c.stats.merged = 40;
        let b = stamina(&c, 30.0);
        c.stats.shifts = 20;
        let d = stamina(&c, 30.0);
        assert!(0 < a && a < b && b < d, "days, merges and shifts each add: {a} {b} {d}");
        c.stats.sick = 3;
        assert_eq!(stamina(&c, 30.0), d - 6, "a little lost each time sick");
        c.stats = CareerStats { merged: 100_000, shifts: 100_000, ..Default::default() };
        assert_eq!(stamina(&c, 10_000.0), 100, "capped");
        c.stats.sick = 1_000;
        assert_eq!(stamina(&c, 0.0), 0, "never below 0");
        rest(&mut c, false);
        rest(&mut c, true);
        assert_eq!((c.stats.shifts, c.stats.sick), (100_001, 1_001));
    }

    #[test]
    fn legends_need_rank_days_merges_and_stamina() {
        let day = 86_400_000;
        let mut c = Career { xp: 5_000, rank: Rank::Staff, ..Default::default() };
        c.stats.merged = 150;
        c.stats.shifts = 45;
        assert!(legend_ok(&c, 90.0), "stamina {}", stamina(&c, 90.0));
        assert!(!legend_ok(&c, 89.0), "90 days on the crew");
        assert!(!legend_ok(&Career { stats: CareerStats { merged: 149, ..c.stats.clone() }, ..c.clone() }, 90.0), "150 merges");
        assert!(!legend_ok(&Career { stats: CareerStats { shifts: 10, ..c.stats.clone() }, ..c.clone() }, 90.0), "stamina 80");
        assert!(!legend_ok(&Career { rank: Rank::Senior, ..c.clone() }, 400.0), "Staff first: XP alone is not enough either way");
        // The core refreshes it on save: once, with a milestone, and the title stays.
        assert!(!refresh(&mut c, 0, 89 * day));
        assert!(!c.legend && c.stamina > 0);
        assert!(refresh(&mut c, 0, 90 * day));
        assert!(c.legend && c.milestones.last().unwrap().text == "Legend");
        c.stats.sick = 20;
        assert!(!refresh(&mut c, 0, 91 * day) && c.legend, "earned for good");
        assert_eq!(c.milestones.iter().filter(|m| m.text == "Legend").count(), 1);
        // Days count from its first milestone when older (an imported agent).
        let old = Career { milestones: vec![Milestone { at: 5, text: "Joined the team".into() }], ..Default::default() };
        assert_eq!(hired_at(&old, 100), 5);
        assert_eq!(hired_at(&Career::default(), 100), 100);
        let mut forged = c.clone();
        forged.stamina = 100;
        let forged = sanitize(forged);
        assert!(!forged.legend && forged.stamina == 0, "an import's title and stamina are recomputed, not read");
    }

    #[test]
    fn some_pieces_are_earned() {
        // The PS5 and the pool table: someone at Senior or above.
        assert_eq!(locked("office.tvStand", &[Rank::Intern, Rank::Junior], []).as_deref(), Some("Someone on the crew must be a Senior"));
        assert_eq!(locked("office.poolTable", &[Rank::Intern, Rank::Staff], []), None);
        assert_eq!(locked("office.pingpong", &[], []).as_deref(), Some("Someone on the crew must be a Junior"));
        assert_eq!(locked("office.foosball", &[], []), None);
        // Coffee: only where there is a kitchen (the garage has none).
        let garage = crate::lot::template("garage").unwrap();
        let items = || garage.furniture.iter().map(|p| p.item.as_str());
        assert!(locked("polypizza.espresso", &[Rank::Staff], items()).unwrap().contains("kitchen"));
        assert_eq!(locked("polypizza.espresso", &[], items().chain(["kenney.kitchenCabinet"])), None);
        let studio = crate::lot::template("studio").unwrap();
        assert_eq!(locked("kenney.kitchenCoffeeMachine", &[], studio.furniture.iter().map(|p| p.item.as_str())), None);
        // Plain pieces need nothing; the shop sells the games but not the desks.
        assert_eq!(locked("kenney.chair", &[], []), None);
        assert!(sold_builtin("office.foosball") && sold_builtin("office.tvStand"));
        assert!(!sold_builtin("office.desk") && !sold_builtin("kenney.chair"));
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
