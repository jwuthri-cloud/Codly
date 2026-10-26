//! Identifier-aware tokenization. The index stores the output of these functions and the
//! query is built from them, so both sides always agree on what a "term" is: FTS5's
//! `unicode61` already splits `io_uring` on the underscore but knows nothing about
//! camelCase, so `retainedSampling` is stored as `retainedsampling retained sampling`.

/// Words with no retrieval signal in code or prose. Dropped on both sides (the index and the
/// query), which also shrinks the postings lists of the commonest tokens.
pub(crate) fn is_noise(t: &str) -> bool {
    matches!(
        t,
        "a" | "an" | "the" | "and" | "or" | "of" | "to" | "in" | "on" | "at" | "by" | "for" | "with" | "from"
            | "as" | "is" | "are" | "was" | "were" | "be" | "been" | "it" | "its" | "this" | "that" | "these"
            | "those" | "if" | "then" | "else" | "not" | "no" | "so" | "we" | "you" | "he" | "she" | "they"
            | "my" | "our" | "your" | "their" | "do" | "does" | "did" | "has" | "have" | "had" | "will"
            | "would" | "can" | "could" | "should" | "may" | "might" | "must" | "there" | "here" | "which"
            | "what" | "when" | "where" | "who" | "how" | "why" | "than" | "too" | "very" | "just" | "into"
            | "out" | "up" | "about" | "after" | "before" | "each" | "any" | "all" | "some" | "such"
            | "only" | "other" | "more" | "most" | "same" | "own" | "but" | "also" | "via"
            // code keywords that appear in a large share of all lines
            | "let" | "var" | "fn" | "func" | "function" | "def" | "pub" | "return" | "self" | "mut" | "int"
            | "void" | "null" | "nil" | "none" | "true" | "false" | "use" | "import" | "const" | "static" | "new"
            | "end" | "begin" | "elif" | "pass" | "lambda"
    )
}

/// Filler verbs of commit subjects and task prompts ("Fix …", "Add … support"). Dropped from
/// *plain* query words only: `add_item` keeps `add` because there it names something.
pub(crate) fn is_generic(t: &str) -> bool {
    matches!(
        t,
        "fix"
            | "fixes"
            | "fixed"
            | "fixing"
            | "add"
            | "adds"
            | "added"
            | "adding"
            | "update"
            | "updates"
            | "updated"
            | "updating"
            | "uses"
            | "used"
            | "using"
            | "make"
            | "makes"
            | "made"
            | "making"
            | "remove"
            | "removes"
            | "removed"
            | "removing"
            | "support"
            | "supports"
            | "supported"
            | "allow"
            | "allows"
            | "allowed"
            | "avoid"
            | "avoids"
            | "keep"
            | "keeps"
            | "move"
            | "moves"
            | "moved"
            | "instead"
            | "improve"
            | "improves"
            | "improved"
            | "implement"
            | "implements"
            | "implemented"
            | "ensure"
            | "ensures"
            | "change"
            | "changes"
            | "changed"
            | "refactor"
            | "refactors"
            | "cleanup"
            | "clean"
            | "bump"
            | "bumps"
            | "need"
            | "needs"
            | "please"
            | "want"
            | "wants"
            | "now"
            | "currently"
            | "again"
            | "still"
            | "bug"
            | "issue"
            | "problem"
    )
}

const MAX_TERM_BYTES: usize = 32;

fn accept(t: &str) -> bool {
    let n = t.len();
    if !(2..=MAX_TERM_BYTES).contains(&n) || is_noise(t) {
        return false;
    }
    // Short numbers are line numbers, indices and flags; long ones (ports, status codes) are kept.
    !(n < 3 && t.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Upper,
    Lower,
    Digit,
}

fn class(c: char) -> Class {
    if c.is_uppercase() {
        Class::Upper
    } else if c.is_ascii_digit() {
        Class::Digit
    } else {
        Class::Lower
    }
}

/// Byte offsets where a camelCase / PascalCase / acronym word boundary falls inside `s`:
/// `fooBar` → [3], `HTTPServer` → [4], `base64Encode` → [6]. Digits stay with the letters
/// before them (`sha256`, `utf8`).
fn camel_boundaries(s: &str, mut f: impl FnMut(usize)) {
    let mut prev: Option<Class> = None;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let cur = class(c);
        if let Some(p) = prev {
            let split = match (p, cur) {
                (Class::Lower | Class::Digit, Class::Upper) => true,
                // `HTTPServer`: the last capital of an acronym starts the next word.
                (Class::Upper, Class::Upper) => it.peek().is_some_and(|&(_, n)| class(n) == Class::Lower),
                _ => false,
            };
            if split {
                f(i);
            }
        }
        prev = Some(cur);
    }
}

/// Calls `f(term, whole, multi)` with every lowercase term of one `_`-free word piece: the
/// whole piece first when it is camelCase (`retainedsampling`, `whole = true`), then its
/// parts (`retained`, `sampling`). `multi` says the piece had more than one part.
fn piece_terms(piece: &str, scratch: &mut String, f: &mut impl FnMut(&str, bool, bool)) {
    let mut cuts: [usize; 16] = [0; 16];
    let mut n = 0;
    camel_boundaries(piece, |i| {
        if n < cuts.len() {
            cuts[n] = i;
            n += 1;
        }
    });
    lower_into(piece, scratch);
    if n == 0 {
        if accept(scratch) {
            f(scratch, false, false);
        }
        return;
    }
    if accept(scratch) {
        f(scratch, true, true);
    }
    let mut start = 0;
    for end in cuts[..n].iter().copied().chain(std::iter::once(piece.len())) {
        lower_into(&piece[start..end], scratch);
        if accept(scratch) {
            f(scratch, false, true);
        }
        start = end;
    }
}

fn lower_into(s: &str, out: &mut String) {
    out.clear();
    if s.is_ascii() {
        out.extend(s.bytes().map(|b| b.to_ascii_lowercase() as char));
    } else {
        out.push_str(&s.to_lowercase());
    }
}

fn is_word_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Calls `f(term, whole, ident)` for every term in `text`, in order. `whole` marks the
/// concatenated form of a camelCase word; `ident` says the word was identifier-shaped
/// (snake_case or camelCase) rather than a plain word.
fn for_each_term(text: &str, mut f: impl FnMut(&str, bool, bool)) {
    let mut scratch = String::with_capacity(32);
    let mut run_start: Option<usize> = None;
    let mut flush = |run: &str, f: &mut dyn FnMut(&str, bool, bool)| {
        let snake = run.contains('_');
        for piece in run.split('_').filter(|p| !p.is_empty()) {
            // A camelCase piece is identifier-shaped even without an underscore.
            piece_terms(piece, &mut scratch, &mut |t, whole, multi| f(t, whole, snake || multi));
        }
    };
    for (i, c) in text.char_indices() {
        if is_word_char(c) {
            run_start.get_or_insert(i);
        } else if let Some(s) = run_start.take() {
            flush(&text[s..i], &mut f);
        }
    }
    if let Some(s) = run_start {
        flush(&text[s..], &mut f);
    }
}

/// Append the index terms of `text` to `out`, space separated. Each line of source is
/// capped so a minified one-liner cannot blow up a chunk.
pub(crate) fn push_terms(text: &str, out: &mut String) {
    let text = if text.len() > 2000 { &text[..floor_char_boundary(text, 2000)] } else { text };
    for_each_term(text, |t, _, _| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(t);
    });
}

pub(crate) fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Index terms of a path: directories, file stem and extension, identifier-split.
pub(crate) fn path_terms(path: &str) -> String {
    let mut s = String::with_capacity(path.len() + 16);
    push_terms(path, &mut s);
    s
}

/// The terms of one query.
#[derive(Debug, Default, Clone)]
pub(crate) struct QueryTerms {
    /// Identifier parts and plain words, deduplicated, in query order. These must co-occur at
    /// the strict levels of the ladder.
    pub required: Vec<String>,
    /// Concatenated camelCase words (`retainedsampling`): strong evidence, but only usable
    /// where a missing term is not fatal (OR levels, the files ranker).
    pub wholes: Vec<String>,
}

impl QueryTerms {
    pub fn is_empty(&self) -> bool {
        self.required.is_empty()
    }
}

/// Terms of a free-text query. URLs are dropped; plain words that are only task filler
/// (`fix`, `add`, `support`…) are dropped unless nothing else remains.
pub(crate) fn query_terms(text: &str) -> QueryTerms {
    let text = strip_urls(text);
    let mut all: Vec<(String, bool, bool)> = Vec::new();
    for_each_term(&text, |t, whole, ident| all.push((t.to_string(), whole, ident)));
    let build = |keep_generic: bool| {
        let mut q = QueryTerms::default();
        for (t, whole, ident) in &all {
            if !keep_generic && !ident && is_generic(t) {
                continue;
            }
            let list = if *whole { &mut q.wholes } else { &mut q.required };
            if !list.contains(t) {
                list.push(t.clone());
            }
        }
        q.required.truncate(24);
        q.wholes.truncate(8);
        q
    };
    let q = build(false);
    if q.is_empty() { build(true) } else { q }
}

fn strip_urls(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("://") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for (i, tok) in text.split(char::is_whitespace).enumerate() {
        if tok.contains("://") {
            continue;
        }
        if i > 0 {
            out.push(' ');
        }
        out.push_str(tok);
    }
    std::borrow::Cow::Owned(out)
}

/// Quote a term for an FTS5 MATCH expression.
pub(crate) fn fts_quote(t: &str) -> String {
    let mut s = String::with_capacity(t.len() + 2);
    s.push('"');
    for c in t.chars() {
        if c == '"' {
            s.push('"');
        }
        s.push(c);
    }
    s.push('"');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(s: &str) -> Vec<String> {
        let mut out = String::new();
        push_terms(s, &mut out);
        out.split(' ').filter(|t| !t.is_empty()).map(String::from).collect()
    }

    #[test]
    fn splits_camel_and_snake() {
        assert_eq!(terms("retainedSampling"), ["retainedsampling", "retained", "sampling"]);
        assert_eq!(terms("io_uring_submit"), ["io", "uring", "submit"]);
        assert_eq!(terms("HTTPServer"), ["httpserver", "http", "server"]);
        assert_eq!(terms("parseURL"), ["parseurl", "parse", "url"]);
        assert_eq!(terms("sha256Hash"), ["sha256hash", "sha256", "hash"]);
    }

    #[test]
    fn drops_noise_and_tiny_numbers() {
        assert_eq!(terms("the 42 of 404 x"), ["404"]);
    }

    #[test]
    fn query_drops_generic_verbs_but_not_identifier_parts() {
        let q = query_terms("Fix retry in upload_client and add_item");
        assert_eq!(q.required, ["retry", "upload", "client", "add", "item"]);
        let q = query_terms("fix the add");
        assert_eq!(q.required, ["fix", "add"], "falls back when only filler remains");
    }

    #[test]
    fn query_separates_wholes() {
        let q = query_terms("retainedSampling guardian");
        assert_eq!(q.required, ["retained", "sampling", "guardian"]);
        assert_eq!(q.wholes, ["retainedsampling"]);
    }
}
