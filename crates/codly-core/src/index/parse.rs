//! One file → everything the index stores about it: symbols, 60-line chunks with their
//! identifier-split terms, path/symbol/doc terms for the files table.

use super::lang;
use super::store::IndexOptions;
use super::symbols::{self, Symbol};
use super::terms;

/// Chunk window and stride: 60 lines, 10 of them shared with the previous chunk so a
/// definition cut by a boundary is whole in one of the two.
const CHUNK_LINES: usize = 60;
const CHUNK_STRIDE: usize = 50;
/// A tail shorter than this is folded into the last chunk instead of becoming its own.
const TAIL_FOLD: usize = 10;
/// Code files longer than this are indexed up to the cap (the rest is almost always
/// generated or data); data files get a much smaller head.
const MAX_LINES: usize = 8000;
const MAX_DATA_LINES: usize = 800;

pub(crate) struct ParsedChunk {
    pub start_line: u32,
    pub end_line: u32,
    pub terms: String,
}

pub(crate) struct ParsedFile {
    pub path: String,
    pub lang: Option<&'static str>,
    pub bytes: u64,
    pub lines: u32,
    pub mtime_ns: i64,
    pub hash: i64,
    pub path_terms: String,
    pub symbol_terms: String,
    pub doc_terms: String,
    pub symbols: Vec<Symbol>,
    pub chunks: Vec<ParsedChunk>,
}

pub(crate) enum Parsed {
    Indexed(Box<ParsedFile>),
    Skipped(&'static str),
}

/// FNV-1a: stable across Rust versions (unlike `DefaultHasher`), which matters because
/// the value is persisted.
pub(crate) fn fnv1a(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h as i64
}

pub(crate) fn parse(path: &str, bytes: &[u8], mtime_ns: i64, opts: &IndexOptions) -> Parsed {
    if let Some(why) = lang::skip_by_name(path) {
        return Parsed::Skipped(why);
    }
    let lang = lang::detect_lang(path);
    // Binary check first: it needs only the head, and it keeps us from decoding images.
    if memchr::memchr(0, &bytes[..bytes.len().min(8192)]).is_some() {
        return Parsed::Skipped("binary");
    }
    let text = String::from_utf8_lossy(bytes);
    let all_lines: Vec<&str> = text.split('\n').collect();
    // `split` yields a trailing empty piece for a final newline; it is not a line.
    let total = if all_lines.last().is_some_and(|l| l.is_empty()) { all_lines.len() - 1 } else { all_lines.len() };
    let max_line = all_lines.iter().map(|l| l.len()).max().unwrap_or(0);
    if let Some(why) = lang::skip_by_content(lang, bytes, &text, total, max_line) {
        return Parsed::Skipped(why);
    }
    let cap = if lang::is_data_lang(lang) { MAX_DATA_LINES } else { MAX_LINES };
    let n = total.min(cap);
    let lines = &all_lines[..n];

    let symbols = match lang {
        Some(l) => symbols::extract(l, &text, &all_lines),
        None => Vec::new(),
    };
    // Keep only symbols inside the indexed window.
    let symbols: Vec<Symbol> = symbols.into_iter().filter(|s| (s.line as usize) <= n).collect();

    let mut symbol_terms = String::new();
    {
        let mut seen: Vec<&str> = Vec::new();
        for s in &symbols {
            // One entry per distinct name: a file with fifty `new` fns is not fifty times as
            // relevant to `new`.
            if seen.contains(&s.name.as_str()) {
                continue;
            }
            seen.push(&s.name);
            terms::push_terms(&s.name, &mut symbol_terms);
            if let Some(c) = &s.container {
                terms::push_terms(c, &mut symbol_terms);
            }
        }
    }

    let doc_terms = doc_terms(lang, lines);
    let line_terms: Vec<String> = lines
        .iter()
        .map(|l| {
            let mut s = String::new();
            terms::push_terms(l, &mut s);
            s
        })
        .collect();
    let path_t = terms::path_terms(path);
    let mut chunks = chunk(&line_terms);
    if opts.path_in_chunks || opts.symbols_in_chunks {
        for c in &mut chunks {
            let mut prefix = String::new();
            if opts.path_in_chunks {
                prefix.push_str(&path_t);
            }
            if opts.symbols_in_chunks {
                // the definitions this chunk sits inside, outermost first
                for s in symbols.iter().filter(|s| s.line < c.start_line && s.end_line >= c.start_line) {
                    terms::push_terms(&s.name, &mut prefix);
                }
            }
            if !prefix.is_empty() {
                c.terms = format!("{prefix} {}", c.terms);
            }
        }
    }

    Parsed::Indexed(Box::new(ParsedFile {
        path: path.to_string(),
        lang,
        bytes: bytes.len() as u64,
        lines: total as u32,
        mtime_ns,
        hash: fnv1a(bytes),
        path_terms: path_t,
        symbol_terms,
        doc_terms,
        symbols,
        chunks,
    }))
}

/// Windows of `CHUNK_LINES` lines at a stride of `CHUNK_STRIDE`; the last one absorbs a short tail.
fn chunk(line_terms: &[String]) -> Vec<ParsedChunk> {
    let n = line_terms.len();
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        let mut end = (start + CHUNK_LINES).min(n);
        if n - end <= TAIL_FOLD {
            end = n;
        }
        let mut t = String::new();
        for l in &line_terms[start..end] {
            if l.is_empty() {
                continue;
            }
            if !t.is_empty() {
                t.push(' ');
            }
            t.push_str(l);
        }
        // Chunks with no terms (blank, punctuation) are not worth a row.
        if !t.is_empty() {
            out.push(ParsedChunk { start_line: (start + 1) as u32, end_line: end as u32, terms: t });
        }
        if end == n {
            break;
        }
        start += CHUNK_STRIDE;
    }
    out
}

/// The leading comment block (or the first lines of a text file): what the file says it is.
fn doc_terms(lang: Option<&str>, lines: &[&str]) -> String {
    let mut out = String::new();
    let mut taken = 0usize;
    let mut i = 0;
    // skip shebang / blank lines / php open tag
    while i < lines.len() {
        let t = lines[i].trim();
        if t.is_empty() || t.starts_with("#!") || t.starts_with("<?php") || t.starts_with("// swift-tools") {
            i += 1;
        } else {
            break;
        }
    }
    if matches!(lang, Some("markdown" | "text")) {
        for l in lines.iter().skip(i).take(15) {
            terms::push_terms(l, &mut out);
        }
        return out;
    }
    // Python module docstring
    if let Some(l) = lines.get(i) {
        let t = l.trim_start();
        let quote = if t.starts_with("\"\"\"") {
            Some("\"\"\"")
        } else if t.starts_with("'''") {
            Some("'''")
        } else {
            None
        };
        if let Some(q) = quote {
            let mut j = i;
            loop {
                terms::push_terms(lines[j], &mut out);
                let closes = if j == i { t[3..].contains(q) } else { lines[j].contains(q) };
                j += 1;
                if closes || j >= lines.len() || j - i > 25 {
                    break;
                }
            }
            return out;
        }
    }
    let mut in_block = false;
    while i < lines.len() && taken < 25 && out.len() < 1500 {
        let t = lines[i].trim_start();
        let is_comment = in_block
            || t.starts_with("//")
            || t.starts_with('#')
                && !t.starts_with("#[")
                && !t.starts_with("#include")
                && !t.starts_with("#define")
                && !t.starts_with("#!")
            || t.starts_with("--")
            || t.starts_with("/*")
            || t.starts_with("* ")
            || t.starts_with('*') && t.len() == 1
            || t.starts_with(";;")
            || t.starts_with("<!--");
        if !is_comment {
            break;
        }
        if t.starts_with("/*") && !t.contains("*/") {
            in_block = true;
        } else if in_block && t.contains("*/") {
            in_block = false;
        }
        terms::push_terms(t, &mut out);
        taken += 1;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(p: Parsed) -> ParsedFile {
        match p {
            Parsed::Indexed(f) => *f,
            Parsed::Skipped(w) => panic!("skipped: {w}"),
        }
    }

    #[test]
    fn chunks_overlap_and_fold_the_tail() {
        let lines: Vec<String> = (0..130).map(|i| format!("word{i}")).collect();
        let cs = chunk(&lines);
        let spans: Vec<(u32, u32)> = cs.iter().map(|c| (c.start_line, c.end_line)).collect();
        assert_eq!(spans, [(1, 60), (51, 110), (101, 130)]);
        let small: Vec<String> = (0..65).map(|i| format!("w{i}")).collect();
        let cs = chunk(&small);
        assert_eq!(cs.len(), 1, "a 65-line file is one chunk");
        assert_eq!((cs[0].start_line, cs[0].end_line), (1, 65));
    }

    #[test]
    fn parses_symbols_and_terms() {
        let src = "//! Retry policy for uploads.\n\npub fn retainSampling() {}\n";
        let f = indexed(parse("src/net/upload.rs", src.as_bytes(), 1, &IndexOptions::default()));
        assert_eq!(f.lang, Some("rust"));
        assert_eq!(f.symbols.len(), 1);
        assert!(f.symbol_terms.contains("retained") || f.symbol_terms.contains("retain"), "{}", f.symbol_terms);
        assert!(f.symbol_terms.contains("sampling"));
        assert!(f.doc_terms.contains("retry") && f.doc_terms.contains("uploads"), "{}", f.doc_terms);
        assert!(f.path_terms.contains("upload") && f.path_terms.contains("net"));
        assert_eq!(f.chunks.len(), 1);
    }

    #[test]
    fn skips_binary_and_generated() {
        assert!(matches!(parse("a.rs", b"fn a() {}\0\0\0", 0, &IndexOptions::default()), Parsed::Skipped("binary")));
        assert!(matches!(
            parse("a.go", b"// Code generated by x. DO NOT EDIT.\npackage a\n", 0, &IndexOptions::default()),
            Parsed::Skipped("generated")
        ));
        assert!(matches!(parse("Cargo.lock", b"[[package]]\n", 0, &IndexOptions::default()), Parsed::Skipped("lockfile")));
    }

    #[test]
    fn crlf_and_missing_final_newline() {
        let f = indexed(parse("a.py", b"def a():\r\n    return 1", 0, &IndexOptions::default()));
        assert_eq!(f.lines, 2);
        assert_eq!(f.symbols[0].name, "a");
    }
}
