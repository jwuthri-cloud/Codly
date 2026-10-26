//! Regex-based symbol extraction: definitions with line spans, good enough for exact-name
//! lookups and for naming the chunk a citation points into.
//!
//! // ponytail: tree-sitter tags (static grammar crates) replace these regexes in v1.5 once the
//! commit-replay benchmark shows symbols are the bottleneck (CRITIQUE C22, D9).
//!
//! One regex pass per rule over the whole file (multi-line anchors), then a line-ordered pass
//! that assigns containers from indentation and finds each span's last line.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::bytes::{Captures, Regex, RegexBuilder};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Symbol {
    pub name: String,
    pub kind: &'static str,
    /// Enclosing type/module/impl target, when nested (`Index` for `Index::refresh`).
    pub container: Option<String>,
    /// 1-based first line and inclusive last line of the definition.
    pub line: u32,
    pub end_line: u32,
}

#[cfg(test)]
impl Symbol {
    /// `Index::refresh` / `refresh`.
    pub fn qualified(&self) -> String {
        match &self.container {
            Some(c) => format!("{c}::{}", self.name),
            None => self.name.clone(),
        }
    }
}

const MAX_SYMBOLS: usize = 2000;
/// Longest definition we measure; beyond it the span is cut (the chunk ranker still covers it).
const MAX_SPAN: usize = 600;

/// Rewrites a match into `(name, container)`, or drops it.
type Post = fn(&Captures) -> Option<(String, Option<String>)>;

struct Rule {
    re: Regex,
    kind: &'static str,
    /// Opens a scope that following, more indented symbols belong to.
    scope: bool,
    post: Option<Post>,
}

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Brace,
    Indent,
    RubyEnd,
    None,
}

struct Lang {
    rules: Vec<Rule>,
    block: Block,
    /// `'` opens a string (JS, PHP) instead of a char literal / lifetime.
    single_quote_string: bool,
    backtick_string: bool,
}

const ID: &str = r"[A-Za-z_][A-Za-z0-9_]*";
const JSID: &str = r"[A-Za-z_$][A-Za-z0-9_$]*";
const MODS: &str = r"public|protected|private|internal|static|final|abstract|sealed|virtual|override|async|partial|readonly|unsafe|extern|synchronized|native|default|strictfp|new";

fn rule(pat: &str, kind: &'static str, scope: bool) -> Rule {
    let pat = pat.replace("{ID}", ID).replace("{JSID}", JSID).replace("{MODS}", MODS);
    let re = RegexBuilder::new(&pat)
        .multi_line(true)
        .crlf(true)
        .unicode(false)
        .build()
        .unwrap_or_else(|e| panic!("bad symbol regex {pat:?}: {e}"));
    Rule { re, kind, scope, post: None }
}

fn rule_post(pat: &str, kind: &'static str, scope: bool, post: Post) -> Rule {
    Rule { post: Some(post), ..rule(pat, kind, scope) }
}

fn rust() -> Lang {
    Lang {
        rules: vec![
            rule(
                r#"^(?P<ind>[ \t]*)(?:pub(?:\([^)\n]*\))?[ \t]+)?(?:default[ \t]+)?(?:const[ \t]+)?(?:async[ \t]+)?(?:unsafe[ \t]+)?(?:extern[ \t]+"[^"\n]*"[ \t]+)?fn[ \t]+(?P<name>{ID})"#,
                "fn",
                false,
            ),
            rule(
                r"^(?P<ind>[ \t]*)(?:pub(?:\([^)\n]*\))?[ \t]+)?(?:unsafe[ \t]+)?(?P<kw>struct|enum|trait|type|mod)[ \t]+(?P<name>{ID})",
                "",
                true,
            ),
            rule(r"^(?P<ind>[ \t]*)(?:pub(?:\([^)\n]*\))?[ \t]+)?(?P<kw>const|static)[ \t]+(?:mut[ \t]+)?(?P<name>{ID})[ \t]*:", "", false),
            rule(r"^(?P<ind>[ \t]*)macro_rules!\s*(?P<name>{ID})", "macro", false),
            rule_post(r"^(?P<ind>[ \t]*)(?:unsafe[ \t]+)?impl\b(?P<rest>[^{;\r\n]*)", "impl", true, rust_impl),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

/// `impl<T> Trait for Vec<T>` → `Vec`; `impl Foo` → `Foo`.
fn rust_impl(c: &Captures) -> Option<(String, Option<String>)> {
    let rest = std::str::from_utf8(c.name("rest")?.as_bytes()).ok()?.trim();
    let mut s = rest;
    if s.starts_with('<') {
        let mut depth = 0;
        let mut end = None;
        for (i, ch) in s.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        s = s[end?..].trim_start();
    }
    if let Some(i) = s.find(" for ") {
        s = s[i + 5..].trim_start();
    }
    let s = s.trim_start_matches(['&', '*']).trim_start();
    let s = s.strip_prefix("dyn ").unwrap_or(s);
    let end = s.find(|ch: char| ch == '<' || ch.is_whitespace() || ch == '{').unwrap_or(s.len());
    let name = s[..end].rsplit("::").next()?;
    (!name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')).then(|| (name.to_string(), None))
}

fn typescript() -> Lang {
    Lang {
        rules: vec![
            rule(
                r"^(?P<ind>[ \t]*)(?:export[ \t]+)?(?:default[ \t]+)?(?:declare[ \t]+)?(?:abstract[ \t]+)?(?P<kw>class|interface|enum|namespace)[ \t]+(?P<name>{JSID})",
                "",
                true,
            ),
            rule(
                r"^(?P<ind>[ \t]*)(?:export[ \t]+)?(?:declare[ \t]+)?type[ \t]+(?P<name>{JSID})[ \t]*(?:<[^=\r\n]*>)?[ \t]*=",
                "type",
                false,
            ),
            rule(
                r"^(?P<ind>[ \t]*)(?:export[ \t]+)?(?:default[ \t]+)?(?:declare[ \t]+)?(?:async[ \t]+)?function\*?[ \t]*(?P<name>{JSID})",
                "fn",
                false,
            ),
            rule(
                r"^(?P<ind>[ \t]*)(?:export[ \t]+)?(?:declare[ \t]+)?(?:const|let|var)[ \t]+(?P<name>{JSID})[ \t]*(?::[^=\r\n]+)?=[ \t]*(?:async[ \t]*)?(?:function\b|\([^)\r\n]*\)[ \t]*(?::[^=\r\n]+)?=>|{JSID}[ \t]*=>)",
                "fn",
                false,
            ),
            rule(r"^(?P<ind>)export[ \t]+(?:declare[ \t]+)?(?:const|let|var)[ \t]+(?P<name>{JSID})", "const", false),
            rule(
                r"^(?P<ind>[ \t]+)(?:(?:public|private|protected|static|async|readonly|override|abstract|get|set)[ \t]+)*(?P<name>{JSID})[ \t]*(?:<[^>\r\n(]*>)?\([^;\r\n]*\)[ \t]*(?::[^{;=\r\n]+)?\{[ \t]*$",
                "method",
                false,
            ),
        ],
        block: Block::Brace,
        single_quote_string: true,
        backtick_string: true,
    }
}

fn python() -> Lang {
    Lang {
        rules: vec![
            rule(r"^(?P<ind>[ \t]*)(?:async[ \t]+)?def[ \t]+(?P<name>{ID})", "fn", false),
            rule(r"^(?P<ind>[ \t]*)class[ \t]+(?P<name>{ID})", "class", true),
            rule(r"^(?P<ind>)(?P<name>[A-Z][A-Z0-9_]{2,})[ \t]*(?::[^=\r\n]+)?=[^=]", "const", false),
        ],
        block: Block::Indent,
        single_quote_string: true,
        backtick_string: false,
    }
}

fn go() -> Lang {
    Lang {
        rules: vec![
            rule(
                r"^(?P<ind>)func[ \t]+(?:\([ \t]*(?:{ID}[ \t]+)?\*?(?P<recv>{ID})(?:\[[^\]\r\n]*\])?[ \t]*\)[ \t]*)?(?P<name>{ID})",
                "fn",
                false,
            ),
            rule(r"^(?P<ind>)type[ \t]+(?P<name>{ID})[ \t]+(?:\[[^\]\r\n]*\][ \t]+)?(?P<kw>struct|interface)\b", "", true),
            rule(r"^(?P<ind>)type[ \t]+(?P<name>{ID})\b", "type", false),
            rule(r"^(?P<ind>)\t(?P<name>[A-Z][A-Za-z0-9_]*)[ \t]+(?P<kw>struct|interface)\b", "", true),
            rule(r"^(?P<ind>)(?P<kw>const|var)[ \t]+(?P<name>{ID})\b", "", false),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: true,
    }
}

fn swift() -> Lang {
    const PRE: &str = r"(?:@\w+(?:\([^)\r\n]*\))?[ \t]+)*(?:(?:public|private|fileprivate|internal|open|final|indirect|nonisolated|package|static|class|override|mutating|nonmutating|required|convenience|async)[ \t]+)*";
    Lang {
        rules: vec![
            rule(
                &format!(
                    r"^(?P<ind>[ \t]*){PRE}(?P<kw>class|struct|enum|protocol|extension|actor|typealias)[ \t]+(?P<name>[A-Za-z_][A-Za-z0-9_.]*)"
                ),
                "",
                true,
            ),
            rule(&format!(r"^(?P<ind>[ \t]*){PRE}func[ \t]+(?P<name>{{ID}})"), "fn", false),
            rule(&format!(r"^(?P<ind>[ \t]*){PRE}(?P<name>init|deinit)[?!]?[ \t]*[(<]"), "init", false),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

fn java() -> Lang {
    Lang {
        rules: vec![
            rule(
                r"^(?P<ind>[ \t]*)(?:@\w+(?:\([^)\r\n]*\))?[ \t]+)*(?:(?:{MODS})[ \t]+)*(?P<kw>class|interface|enum|record|@interface|struct|namespace)[ \t]+(?P<name>[A-Za-z_][A-Za-z0-9_.]*)",
                "",
                true,
            ),
            rule(
                r"^(?P<ind>[ \t]+)(?:@\w+(?:\([^)\r\n]*\))?[ \t]+)*(?:(?:{MODS})[ \t]+)+(?:<[^>\r\n]+>[ \t]+)?[\w.<>\[\],?& \t]+?[ \t]+(?P<name>{ID})[ \t]*\(",
                "method",
                false,
            ),
            rule(
                r"^(?P<ind>[ \t]+)(?:@\w+(?:\([^)\r\n]*\))?[ \t]+)*(?:public|protected|private|internal)[ \t]+(?P<name>[A-Z][A-Za-z0-9_]*)[ \t]*\(",
                "method",
                false,
            ),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

fn kotlin() -> Lang {
    const PRE: &str = r"(?:@\w+(?:\([^)\r\n]*\))?[ \t]+)*(?:(?:public|private|protected|internal|open|abstract|final|override|suspend|inline|operator|infix|data|sealed|enum|annotation|companion|tailrec|external|const|lateinit|value|inner|actual|expect)[ \t]+)*";
    Lang {
        rules: vec![
            rule(
                &format!(
                    r"^(?P<ind>[ \t]*){PRE}(?P<kw>class|interface|object|fun|typealias)[ \t]+(?:<[^>\r\n]+>[ \t]+)?(?:[\w.<>?]+\.)?(?P<name>{{ID}})"
                ),
                "",
                true,
            ),
            rule(&format!(r"^(?P<ind>[ \t]{{0,4}}){PRE}(?P<kw>val|var)[ \t]+(?P<name>{{ID}})"), "", false),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

fn scala() -> Lang {
    Lang {
        rules: vec![rule(
            r"^(?P<ind>[ \t]*)(?:(?:private|protected|final|sealed|abstract|case|implicit|override|lazy)[ \t]+)*(?P<kw>class|object|trait|def|type|val)[ \t]+(?P<name>{ID})",
            "",
            true,
        )],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

fn ruby() -> Lang {
    Lang {
        rules: vec![
            rule(r"^(?P<ind>[ \t]*)def[ \t]+(?:self\.|[A-Z]\w*\.)?(?P<name>[A-Za-z_]\w*[?!=]?)", "fn", false),
            rule(r"^(?P<ind>[ \t]*)(?P<kw>class|module)[ \t]+(?P<name>[A-Z][\w:]*)", "", true),
            rule(r"^(?P<ind>[ \t]*)(?P<name>[A-Z][A-Z0-9_]+)[ \t]*=[^=~]", "const", false),
        ],
        block: Block::RubyEnd,
        single_quote_string: true,
        backtick_string: false,
    }
}

fn c_family(cpp: bool) -> Lang {
    let mut rules = vec![
        rule(
            r"^(?P<ind>[ \t]*)(?:typedef[ \t]+)?(?:template[ \t]*<[^>\r\n]*>[ \t]*)?(?P<kw>struct|union|enum[ \t]+class|enum|class|namespace)[ \t]+(?:\[\[[^\]\r\n]*\]\][ \t]+)?(?P<name>{ID})[ \t]*(?:\{|:[^:]|$|//|/\*)",
            "",
            true,
        ),
        rule(r"^(?P<ind>)[ \t]*#[ \t]*define[ \t]+(?P<name>{ID})", "macro", false),
        rule_post(r"^(?P<ind>)(?P<pre>[A-Za-z_][\w \t*&:<>,~]*?[ \t*&])(?P<name>[A-Za-z_~][\w:~]*)[ \t]*\(", "fn", false, c_function),
        rule_post(r"^(?P<ind>)(?P<name>[A-Za-z_]\w*)\([^;\r\n]*$", "fn", false, c_function_bare),
    ];
    if cpp {
        rules.push(rule_post(r"^(?P<ind>[ \t]+)(?:(?:virtual|static|inline|explicit|constexpr|friend|extern)[ \t]+)+[\w:<>,*&~ \t]*?[ \t*&](?P<name>~?[A-Za-z_]\w*)[ \t]*\(", "method", false, c_function_bare));
    }
    Lang { rules, block: Block::Brace, single_quote_string: false, backtick_string: false }
}

const C_NOT_FUNCTION: &[&str] = &[
    "if",
    "else",
    "while",
    "for",
    "switch",
    "return",
    "sizeof",
    "defined",
    "case",
    "goto",
    "do",
    "typedef",
    "using",
    "namespace",
    "delete",
    "throw",
    "new",
    "catch",
    "try",
    "__attribute__",
    "static_assert",
    "assert",
    "alignas",
    "alignof",
    "decltype",
];

fn c_ident_ok(name: &str) -> bool {
    !C_NOT_FUNCTION.contains(&name)
        // ALL_CAPS(…) at top level is a macro invocation, not a function.
        && !(name.len() >= 3 && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'))
}

fn c_function(c: &Captures) -> Option<(String, Option<String>)> {
    let pre = std::str::from_utf8(c.name("pre")?.as_bytes()).ok()?;
    let first = pre.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).find(|w| !w.is_empty())?;
    if C_NOT_FUNCTION.contains(&first) || pre.contains("=") {
        return None;
    }
    c_function_bare(c)
}

fn c_function_bare(c: &Captures) -> Option<(String, Option<String>)> {
    let raw = std::str::from_utf8(c.name("name")?.as_bytes()).ok()?;
    let (container, name) = match raw.rsplit_once("::") {
        Some((q, n)) => (Some(q.rsplit("::").next().unwrap_or(q).to_string()), n),
        None => (None, raw),
    };
    c_ident_ok(name.trim_start_matches('~')).then(|| (name.to_string(), container))
}

fn php() -> Lang {
    Lang {
        rules: vec![
            rule(
                r"^(?P<ind>[ \t]*)(?:(?:public|protected|private|static|final|abstract)[ \t]+)*function[ \t]+&?(?P<name>{ID})",
                "fn",
                false,
            ),
            rule(r"^(?P<ind>[ \t]*)(?:(?:final|abstract)[ \t]+)*(?P<kw>class|interface|trait|enum)[ \t]+(?P<name>{ID})", "", true),
        ],
        block: Block::Brace,
        single_quote_string: true,
        backtick_string: false,
    }
}

fn shell() -> Lang {
    Lang {
        rules: vec![rule(r"^(?P<ind>)(?:function[ \t]+)?(?P<name>[A-Za-z_][\w-]*)[ \t]*\(\)[ \t]*\{?", "fn", false)],
        block: Block::Brace,
        single_quote_string: true,
        backtick_string: false,
    }
}

fn lua() -> Lang {
    Lang {
        rules: vec![rule(r"^(?P<ind>[ \t]*)(?:local[ \t]+)?function[ \t]+(?P<name>[\w.:]+)", "fn", false)],
        block: Block::RubyEnd,
        single_quote_string: true,
        backtick_string: false,
    }
}

fn sql() -> Lang {
    let mut l = Lang { rules: vec![], block: Block::None, single_quote_string: true, backtick_string: false };
    let re = RegexBuilder::new(r#"^[ \t]*create[ \t]+(?:or[ \t]+replace[ \t]+)?(?:unique[ \t]+)?(?:virtual[ \t]+)?(?P<kw>table|view|function|procedure|index|trigger)[ \t]+(?:if[ \t]+not[ \t]+exists[ \t]+)?(?P<name>[\w."]+)"#)
        .multi_line(true)
        .case_insensitive(true)
        .unicode(false)
        .build()
        .expect("sql regex");
    l.rules.push(Rule { re, kind: "", scope: false, post: None });
    l
}

fn proto() -> Lang {
    Lang {
        rules: vec![
            rule(r"^(?P<ind>[ \t]*)(?P<kw>message|service|enum)[ \t]+(?P<name>{ID})", "", true),
            rule(r"^(?P<ind>[ \t]*)rpc[ \t]+(?P<name>{ID})[ \t]*\(", "fn", false),
        ],
        block: Block::Brace,
        single_quote_string: false,
        backtick_string: false,
    }
}

static LANGS: LazyLock<HashMap<&'static str, Lang>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("rust", rust());
    for l in ["typescript", "javascript", "svelte", "vue"] {
        m.insert(l, typescript());
    }
    m.insert("python", python());
    m.insert("go", go());
    m.insert("swift", swift());
    for l in ["java", "csharp"] {
        m.insert(l, java());
    }
    m.insert("kotlin", kotlin());
    m.insert("scala", scala());
    m.insert("ruby", ruby());
    for l in ["c", "objc"] {
        m.insert(l, c_family(false));
    }
    m.insert("cpp", c_family(true));
    m.insert("php", php());
    m.insert("shell", shell());
    m.insert("lua", lua());
    m.insert("sql", sql());
    m.insert("proto", proto());
    m
});

fn intern_kind(raw: &[u8]) -> &'static str {
    match raw {
        b"fn" | b"fun" | b"func" | b"def" | b"function" => "fn",
        b"struct" => "struct",
        b"enum" => "enum",
        b"trait" => "trait",
        b"type" | b"typealias" => "type",
        b"mod" | b"module" | b"namespace" => "mod",
        b"class" => "class",
        b"interface" | b"protocol" => "interface",
        b"object" => "object",
        b"extension" => "extension",
        b"actor" => "actor",
        b"record" => "record",
        b"union" => "union",
        b"const" => "const",
        b"static" => "static",
        b"val" | b"var" | b"let" => "var",
        b"message" => "message",
        b"service" => "service",
        b"table" => "table",
        b"view" => "view",
        b"procedure" => "fn",
        b"index" => "index",
        b"trigger" => "trigger",
        b"@interface" => "interface",
        _ if raw.starts_with(b"enum") => "enum",
        _ => "symbol",
    }
}

struct Raw {
    line: usize,
    indent: usize,
    name: String,
    kind: &'static str,
    container: Option<String>,
    scope: bool,
}

fn indent_of(s: &[u8]) -> usize {
    let mut n = 0;
    for &b in s {
        match b {
            b' ' => n += 1,
            b'\t' => n += 4,
            _ => break,
        }
    }
    n
}

const JS_NOT_METHOD: &[&str] =
    &["if", "for", "while", "switch", "catch", "with", "return", "else", "do", "try", "function", "typeof", "await", "super"];

/// All definitions in `text`. `lines` are `text` split at `\n`.
pub(crate) fn extract(lang: &str, text: &str, lines: &[&str]) -> Vec<Symbol> {
    // `split('\n')` leaves an empty piece after a final newline; it is not a line.
    let lines = match lines.split_last() {
        Some((&"", rest)) => rest,
        _ => lines,
    };
    if lang == "markdown" {
        return markdown_headings(lines);
    }
    let Some(l) = LANGS.get(lang) else { return Vec::new() };
    let bytes = text.as_bytes();
    let mut line_starts: Vec<usize> = vec![0];
    line_starts.extend(memchr::memchr_iter(b'\n', bytes).map(|i| i + 1));
    let line_of = |off: usize| line_starts.partition_point(|&s| s <= off);

    let mut raws: Vec<Raw> = Vec::new();
    for r in &l.rules {
        for c in r.re.captures_iter(bytes) {
            let whole = c.get(0).expect("group 0");
            let line = line_of(whole.start());
            let mut name: Option<String> = c.name("name").map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned());
            let mut container = c.name("recv").map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned());
            if let Some(post) = r.post {
                match post(&c) {
                    Some((n, ct)) => {
                        name = Some(n);
                        if ct.is_some() {
                            container = ct;
                        }
                    }
                    None => continue,
                }
            }
            let Some(name) = name else { continue };
            if lang_is_js(lang) && r.kind == "method" && JS_NOT_METHOD.contains(&name.as_str()) {
                continue;
            }
            let kind = match c.name("kw") {
                Some(kw) => intern_kind(kw.as_bytes()),
                None => r.kind,
            };
            // Go: methods carry their receiver; mark them so the display reads `Recv::method`.
            raws.push(Raw { line, indent: c.name("ind").map_or(0, |m| indent_of(m.as_bytes())), name, kind, container, scope: r.scope });
        }
    }
    raws.sort_by(|a, b| a.line.cmp(&b.line).then(b.scope.cmp(&a.scope)));
    raws.dedup_by(|b, a| a.line == b.line && a.name == b.name);
    raws.truncate(MAX_SYMBOLS);

    // Containers from indentation: a symbol belongs to the nearest preceding scope that is
    // less indented. Explicit containers (Go receivers, C++ `Foo::bar`, Rust impl targets) win.
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut out: Vec<Symbol> = Vec::with_capacity(raws.len());
    for r in &raws {
        while stack.last().is_some_and(|(ind, _)| *ind >= r.indent) {
            stack.pop();
        }
        let container = r.container.clone().or_else(|| stack.last().map(|(_, n)| n.clone()));
        // ponytail: an `impl` header is both a symbol and the scope for its methods.
        if r.scope {
            stack.push((r.indent, r.name.clone()));
        }
        let end = match l.block {
            Block::Brace => brace_end(lines, r.line - 1, l),
            Block::Indent => indent_end(lines, r.line - 1, r.indent),
            Block::RubyEnd => end_keyword_end(lines, r.line - 1, r.indent),
            Block::None => r.line - 1,
        };
        out.push(Symbol { name: r.name.clone(), kind: r.kind, container, line: r.line as u32, end_line: (end + 1).max(r.line) as u32 });
    }
    out
}

fn lang_is_js(lang: &str) -> bool {
    matches!(lang, "typescript" | "javascript" | "svelte" | "vue")
}

/// Markdown headings as symbols (so docs are searchable by section title). Fenced code is
/// skipped because `# comment` lines in shell/python blocks look like headings.
fn markdown_headings(lines: &[&str]) -> Vec<Symbol> {
    let mut out: Vec<(usize, usize, String)> = Vec::new(); // (line index, level, title)
    let mut fence = false;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || !t.starts_with('#') {
            continue;
        }
        let level = t.bytes().take_while(|&b| b == b'#').count();
        if level > 6 {
            continue;
        }
        let title = t[level..].trim().trim_end_matches('#').trim();
        if !title.is_empty() && t.as_bytes().get(level).is_some_and(|b| *b == b' ' || *b == b'\t') {
            out.push((i, level, title.chars().take(80).collect()));
        }
    }
    let mut syms = Vec::with_capacity(out.len());
    for (k, (i, level, title)) in out.iter().enumerate() {
        let end = out[k + 1..].iter().find(|(_, l2, _)| l2 <= level).map_or(lines.len(), |(j, _, _)| *j);
        syms.push(Symbol {
            name: title.clone(),
            kind: "heading",
            container: None,
            line: (*i + 1) as u32,
            end_line: end.max(*i + 1) as u32,
        });
    }
    syms.truncate(MAX_SYMBOLS);
    syms
}

#[derive(Clone, Copy)]
enum State {
    Code,
    Block,
    Str(u8),
}

/// Last line (0-based) of the brace-delimited definition starting on line `start`.
/// A small lexer (comments, strings, char literals) keeps braces inside them from counting.
fn brace_end(lines: &[&str], start: usize, lang: &Lang) -> usize {
    let limit = (start + MAX_SPAN).min(lines.len());
    let mut depth = 0i32;
    let mut paren = 0i32;
    let mut opened = false;
    let mut st = State::Code;
    for (i, line) in lines.iter().enumerate().take(limit).skip(start) {
        let b = line.as_bytes();
        let mut j = 0;
        while j < b.len() {
            match st {
                State::Code => match b[j] {
                    b'/' if b.get(j + 1) == Some(&b'/') => break,
                    b'#' if lang.single_quote_string && !lang.backtick_string => break, // php/shell comment
                    b'/' if b.get(j + 1) == Some(&b'*') => {
                        st = State::Block;
                        j += 1;
                    }
                    b'"' => st = State::Str(b'"'),
                    b'`' if lang.backtick_string => st = State::Str(b'`'),
                    b'\'' => {
                        if lang.single_quote_string {
                            st = State::Str(b'\'');
                        } else if b.get(j + 1) == Some(&b'\\') {
                            // '\n', '\'' …: skip to the closing quote
                            if let Some(k) = b[j + 2..].iter().position(|&c| c == b'\'') {
                                j += 2 + k;
                            }
                        } else if b.get(j + 2) == Some(&b'\'') {
                            j += 2; // 'x'
                        } // else a lifetime or apostrophe
                    }
                    b'{' => {
                        depth += 1;
                        opened = true;
                    }
                    b'}' => {
                        depth -= 1;
                        if opened && depth <= 0 {
                            return i;
                        }
                    }
                    b'(' => paren += 1,
                    b')' => paren -= 1,
                    b';' if !opened && paren <= 0 => return i, // declaration without a body
                    _ => {}
                },
                State::Block => {
                    if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        st = State::Code;
                        j += 1;
                    }
                }
                State::Str(q) => match b[j] {
                    b'\\' => j += 1,
                    c if c == q => st = State::Code,
                    _ => {}
                },
            }
            j += 1;
        }
        if matches!(st, State::Str(q) if q != b'`') {
            st = State::Code; // plain strings do not span lines
        }
    }
    limit.saturating_sub(1).max(start)
}

/// Python: header (possibly several lines) then every following line indented deeper.
fn indent_end(lines: &[&str], start: usize, indent: usize) -> usize {
    let mut i = start;
    let mut depth = 0i32;
    'header: loop {
        for ch in lines[i].bytes() {
            match ch {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'#' => break,
                b':' if depth <= 0 => break 'header,
                _ => {}
            }
        }
        i += 1;
        if i >= lines.len() || i - start > 40 {
            return start;
        }
    }
    let mut end = i;
    // `def f(): return 1` — one-liner
    let rest = lines[i].trim_end();
    if !rest.ends_with(':') && !rest.contains("#") {
        return i;
    }
    for (j, l) in lines.iter().enumerate().take((start + MAX_SPAN).min(lines.len())).skip(i + 1) {
        if l.trim().is_empty() {
            continue;
        }
        if indent_of(l.as_bytes()) > indent {
            end = j;
        } else {
            break;
        }
    }
    end
}

/// Ruby/Lua: the first later line at the same indentation that starts with `end`.
fn end_keyword_end(lines: &[&str], start: usize, indent: usize) -> usize {
    let limit = (start + MAX_SPAN).min(lines.len());
    for (j, l) in lines.iter().enumerate().take(limit).skip(start + 1) {
        let t = l.trim_start();
        if indent_of(l.as_bytes()) == indent
            && (t == "end" || t.starts_with("end ") || t.starts_with("end;") || t.starts_with("end)") || t.starts_with("end."))
        {
            return j;
        }
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syms(lang: &str, src: &str) -> Vec<Symbol> {
        let lines: Vec<&str> = src.split('\n').collect();
        extract(lang, src, &lines)
    }

    fn names(v: &[Symbol]) -> Vec<String> {
        v.iter().map(|s| format!("{} {}", s.kind, s.qualified())).collect()
    }

    #[test]
    fn rust_items_and_containers() {
        let src = "pub struct Index {\n    a: u8,\n}\n\nimpl<T> Index<T> {\n    pub async fn refresh(&self) -> u8 {\n        if x { 1 } else { 2 }\n    }\n}\n\nconst MAX: usize = 3;\nmacro_rules! boom { () => {} }\n";
        let s = syms("rust", src);
        assert_eq!(names(&s), ["struct Index", "impl Index", "fn Index::refresh", "const MAX", "macro boom"]);
        let refresh = s.iter().find(|x| x.name == "refresh").unwrap();
        assert_eq!((refresh.line, refresh.end_line), (6, 8));
        let st = &s[0];
        assert_eq!((st.line, st.end_line), (1, 3));
    }

    #[test]
    fn rust_trait_impl_target() {
        let s = syms("rust", "impl fmt::Display for Foo<'a> {\n    fn fmt(&self) {}\n}\n");
        assert_eq!(names(&s), ["impl Foo", "fn Foo::fmt"]);
    }

    #[test]
    fn brace_scan_ignores_strings_and_comments() {
        let src = "fn a() {\n    let s = \"}\";\n    // }\n    let c = '}';\n    /* } */\n}\nfn b() {}\n";
        let s = syms("rust", src);
        assert_eq!((s[0].line, s[0].end_line), (1, 6));
        assert_eq!((s[1].line, s[1].end_line), (7, 7));
    }

    #[test]
    fn typescript_forms() {
        let src = "export interface Foo {\n  a: string;\n}\nexport const useThing = async (x: number) => {\n  return x;\n};\nexport class Bar extends Foo {\n  private load(id: string): void {\n    if (id) {\n    }\n  }\n}\nfunction helper() {}\nexport type Id = string;\n";
        let s = syms("typescript", src);
        assert_eq!(names(&s), ["interface Foo", "fn useThing", "class Bar", "method Bar::load", "fn helper", "type Id"]);
        assert_eq!(s[1].end_line, 6);
    }

    #[test]
    fn python_spans() {
        let src = "import os\n\nMAX_RETRIES = 3\n\nclass Client:\n    def __init__(self,\n                 a):\n        self.a = a\n\n    def send(self):\n        return 1\n\ndef top(): return 1\n";
        let s = syms("python", src);
        assert_eq!(names(&s), ["const MAX_RETRIES", "class Client", "fn Client::__init__", "fn Client::send", "fn top"]);
        let client = &s[1];
        assert_eq!((client.line, client.end_line), (5, 11));
        assert_eq!((s[2].line, s[2].end_line), (6, 8));
        assert_eq!((s[4].line, s[4].end_line), (13, 13));
    }

    #[test]
    fn go_methods_carry_receivers() {
        let src =
            "package x\n\ntype Server struct {\n\tn int\n}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n\nfunc helper() {}\n";
        let s = syms("go", src);
        assert_eq!(names(&s), ["struct Server", "fn Server::Start", "fn helper"]);
    }

    #[test]
    fn swift_java_kotlin_ruby() {
        let sw = syms("swift", "public final class Store {\n    public func load(id: Int) {\n    }\n    init(x: Int) {}\n}\nstruct P {}\n");
        assert_eq!(names(&sw), ["class Store", "fn Store::load", "init Store::init", "struct P"]);
        let jv = syms(
            "java",
            "public class Foo {\n    private static final Logger LOG = make(Foo.class);\n    public static int parse(String s) {\n        return 1;\n    }\n    public Foo(int x) {\n    }\n}\n",
        );
        assert_eq!(names(&jv), ["class Foo", "method Foo::parse", "method Foo::Foo"]);
        let kt = syms("kotlin", "class Repo(val db: Db) {\n    suspend fun fetch(id: Int): X {\n        return x\n    }\n}\nobject Util\n");
        assert_eq!(names(&kt), ["class Repo", "fn Repo::fetch", "object Util"]);
        let rb = syms("ruby", "module Billing\n  class Invoice\n    def total?\n      1\n    end\n  end\nend\n");
        assert_eq!(names(&rb), ["mod Billing", "class Billing::Invoice", "fn Invoice::total?"]);
        assert_eq!((rb[2].line, rb[2].end_line), (3, 5));
    }

    #[test]
    fn c_headers() {
        let src = "#define MAX_LEN 10\n#ifndef X\nstruct sk_buff {\n    int len;\n};\nstatic inline int skb_len(const struct sk_buff *skb);\nvoid *alloc_thing(size_t n)\n{\n    return foo(n);\n}\nEXPORT_SYMBOL(alloc_thing);\nint\nlinux_style(int a)\n{\n}\n";
        let s = syms("c", src);
        let n = names(&s);
        assert!(n.contains(&"macro MAX_LEN".to_string()), "{n:?}");
        assert!(n.contains(&"struct sk_buff".to_string()), "{n:?}");
        assert!(n.contains(&"fn skb_len".to_string()), "{n:?}");
        assert!(n.contains(&"fn alloc_thing".to_string()), "{n:?}");
        assert!(n.contains(&"fn linux_style".to_string()), "{n:?}");
        assert!(!n.iter().any(|x| x.contains("EXPORT_SYMBOL") || x.contains("foo")), "{n:?}");
    }

    #[test]
    fn markdown_sections() {
        let s = syms("markdown", "# Title\n\n## Setup\n```sh\n# not a heading\n```\n\n## Usage\ntext\n");
        assert_eq!(names(&s), ["heading Title", "heading Setup", "heading Usage"]);
        assert_eq!((s[0].line, s[0].end_line), (1, 9));
        assert_eq!((s[1].line, s[1].end_line), (3, 7));
    }
}
