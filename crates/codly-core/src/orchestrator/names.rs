//! Friendly names and identity colors for agent characters. Names are short, neutral and
//! deliberately not taken from TV casts (Munder's office cast is the counter-example).

/// Number of identity hues (`--id-1`..`--id-8`); `Agent.color` is an index into them.
pub const COLORS: i64 = 8;

const NAMES: &[&str] = &[
    "Ada", "Bo", "Cleo", "Dax", "Eli", "Fern", "Gus", "Hana", "Ivo", "Juno", "Kai", "Lark", "Milo", "Nia", "Otto", "Pia", "Quin", "Remy",
    "Sol", "Tess", "Uma", "Vik", "Wren", "Yara", "Zeno", "Alba", "Bram", "Cora", "Dov", "Esme", "Finn", "Gwen",
];

/// The first name not already taken in the workspace (case-insensitive); numbered once all are used.
pub fn next_name(taken: &[String]) -> String {
    let used = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if let Some(n) = NAMES.iter().find(|n| !used(n)) {
        return n.to_string();
    }
    (2..).flat_map(|i| NAMES.iter().map(move |n| format!("{n} {i}"))).find(|n| !used(n)).unwrap_or_else(|| "Agent".into())
}

/// The first identity color no agent uses yet, else round-robin.
pub fn next_color(used: &[i64]) -> i64 {
    (0..COLORS).find(|c| !used.contains(c)).unwrap_or(used.len() as i64 % COLORS)
}

/// Lowercase ASCII slug for branch and path names ("Add dark mode!" → "add-dark-mode").
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "x".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_colors_slugs() {
        assert_eq!(next_name(&[]), "Ada");
        assert_eq!(next_name(&["ada".into(), "Bo".into()]), "Cleo");
        let all: Vec<String> = NAMES.iter().map(|n| n.to_string()).collect();
        assert_eq!(next_name(&all), "Ada 2");
        assert_eq!(next_color(&[0, 1, 3]), 2);
        assert_eq!(next_color(&(0..8).collect::<Vec<_>>()), 0);
        assert_eq!(slug("Add dark mode!", 24), "add-dark-mode");
        assert_eq!(slug("¿?", 10), "x");
    }
}
