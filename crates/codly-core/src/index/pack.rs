//! The pointer pack: what goes into a task prompt. Pointers, not content (every injected
//! token is re-paid on every later turn), and nothing but `rg` hints when the index is not
//! confident: "when unsure, inject less" (context-engineering §4.6, Cognition's F0.5 lesson).
// ponytail: no snippet of the top span at `High` confidence yet (§4.8); add it when the replay
// can score Span-Recall@budget.

use crate::model::Confidence;

use super::query::{Ranked, confidence_label};

/// ≈ 1.2k tokens at 4 chars/token.
const MAX_CHARS: usize = 4800;
const MAX_LINE_CHARS: usize = 230;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One line per citation: `[C1] path:12-70 fn Index::refresh — why (confidence)`.
fn line(r: &Ranked, i: usize) -> String {
    let c = &r.result.citations[i];
    let m = &r.meta[i];
    let mut s = format!("[{}] {}:{}-{}", c.id, c.path, c.start_line, c.end_line);
    if let Some(sym) = &c.symbol {
        s.push(' ');
        if let Some(k) = &m.kind {
            s.push_str(k);
            s.push(' ');
        }
        s.push_str(sym);
    }
    let why = c.why.iter().take(2).cloned().collect::<Vec<_>>().join("; ");
    if !why.is_empty() {
        s.push_str(" — ");
        s.push_str(&why);
    }
    s.push_str(&format!(" ({})", confidence_label(m.level)));
    clip(&s, MAX_LINE_CHARS)
}

/// Citations worth injecting at this confidence: up to 6 at `Exact`, 5 at `High`, 8 at `Medium`
/// (never one the ranking itself rates `Low`), none at `Low`.
fn selected(r: &Ranked) -> Vec<usize> {
    let cap = match r.result.confidence {
        Confidence::Exact => 6,
        Confidence::High => 5,
        Confidence::Medium => 8,
        Confidence::Low => 0,
    };
    (0..r.result.citations.len()).filter(|&i| r.meta[i].level != Confidence::Low).take(cap).collect()
}

pub(crate) fn render(r: &Ranked) -> String {
    let res = &r.result;
    let q = clip(&res.query.split_whitespace().collect::<Vec<_>>().join(" "), 90);
    let conf = confidence_label(res.confidence);
    let mut out = String::new();
    let picks = selected(r);
    if res.confidence == Confidence::Low || picks.is_empty() {
        out.push_str(&format!("Codly code search (\"{q}\"): low confidence, no pointers injected. Search yourself, e.g.:\n"));
        if res.fallback_rg.is_empty() {
            out.push_str("  rg -n -i <the rarest words of the task>\n");
        }
        for p in res.fallback_rg.iter().take(3) {
            out.push_str(&format!("  rg -n -i '{p}'\n"));
        }
        return out;
    }
    out.push_str(&format!(
        "Codly code pointers (\"{q}\", confidence: {conf}). Pointers only: open the spans you need and verify before relying on them.\n"
    ));
    for i in picks {
        let l = line(r, i);
        if out.len() + l.len() + 1 > MAX_CHARS {
            break;
        }
        out.push_str(&l);
        out.push('\n');
    }
    if res.confidence == Confidence::Medium && !res.fallback_rg.is_empty() {
        let hint = format!("If these miss: rg -n -i '{}'\n", res.fallback_rg[0]);
        if out.len() + hint.len() <= MAX_CHARS {
            out.push_str(&hint);
        }
    }
    out
}
