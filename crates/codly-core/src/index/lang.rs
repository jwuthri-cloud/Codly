//! Language detection by extension and the "is this worth indexing" filters (lockfiles,
//! minified bundles, generated code, binaries).

use std::path::Path;

/// Directories that are never source in any ecosystem: version-control data, installed
/// dependencies, tool caches and build state. They are cut everywhere, and no ignore rule
/// brings them back.
pub(crate) fn is_denied_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | ".tox"
            | ".mypy_cache"
            | ".pytest_cache"
            | ".ruff_cache"
            | ".gradle"
            | ".idea"
            | "DerivedData"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | ".turbo"
            | ".parcel-cache"
            | "bower_components"
            | ".direnv"
            | ".terraform"
    )
}

/// Names that hold build output or third-party code in most repos, but first-party code in
/// some (a `vendor` domain package, a `dist` or `coverage` module). `.gitignore` already hides
/// most of them in a git checkout; this is the net for repos that do not list them. The ignore
/// files decide first: a `!` rule in `.gitignore`, `.ignore` or `.codlyignore` (`!/src/vendor/`)
/// brings such a directory back.
pub(crate) fn is_default_excluded_dir(name: &str) -> bool {
    matches!(name, "target" | "venv" | "Pods" | "vendor" | "dist" | ".build" | ".cache" | "coverage")
}

/// Language label from the file name, `None` for unknown text (still indexed as text).
pub(crate) fn detect_lang(path: &str) -> Option<&'static str> {
    let p = Path::new(path);
    let name = p.file_name()?.to_str()?;
    match name {
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make"),
        "Dockerfile" | "Containerfile" => return Some("dockerfile"),
        "CMakeLists.txt" => return Some("cmake"),
        "Rakefile" | "Gemfile" | "Podfile" | "Fastfile" => return Some("ruby"),
        "Justfile" | "justfile" => return Some("just"),
        _ => {}
    }
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "svelte" => "svelte",
        "vue" => "vue",
        "py" | "pyi" | "pyw" => "python",
        "go" => "go",
        "swift" => "swift",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "scala" | "sc" => "scala",
        "rb" | "rake" | "gemspec" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ipp" | "inl" | "mm" => "cpp",
        "m" => "objc",
        "cs" => "csharp",
        "php" => "php",
        "lua" => "lua",
        "sh" | "bash" | "zsh" | "fish" => "shell",
        "sql" => "sql",
        "md" | "mdx" | "markdown" | "rst" | "adoc" => "markdown",
        "json" | "jsonc" | "json5" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "xml" | "plist" | "xib" | "storyboard" => "xml",
        "proto" => "proto",
        "graphql" | "gql" => "graphql",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "dart" => "dart",
        "txt" => "text",
        "csv" | "tsv" | "ndjson" | "jsonl" => "data",
        _ => return None,
    })
}

/// Files whose content is data rather than code: kept, but only their head is indexed.
pub(crate) fn is_data_lang(lang: Option<&str>) -> bool {
    matches!(lang, Some("data" | "json" | "xml" | "text"))
}

/// Cheap reject by name before touching the file.
pub(crate) fn skip_by_name(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "cargo.lock"
            | "poetry.lock"
            | "gemfile.lock"
            | "composer.lock"
            | "go.sum"
            | "podfile.lock"
            | "pdm.lock"
            | "uv.lock"
            | "bun.lock"
            | "flake.lock"
            | "package.resolved"
            | "mix.lock"
            | "pubspec.lock"
            | ".ds_store"
    ) || lower.ends_with(".lock")
    {
        return Some("lockfile");
    }
    for suffix in [
        ".min.js",
        ".min.css",
        ".min.mjs",
        ".map",
        ".snap",
        ".svg",
        ".pbxproj",
        ".xcworkspacedata",
        ".bundle.js",
        ".pb.go",
        ".pb.cc",
        ".pb.h",
        "_pb2.py",
        "_pb2_grpc.py",
    ] {
        if lower.ends_with(suffix) {
            return Some("generated");
        }
    }
    const BINARY: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "tiff", "heic", "pdf", "zip", "gz", "tgz", "bz2", "xz", "zst", "7z",
        "rar", "jar", "war", "class", "so", "dylib", "dll", "exe", "o", "a", "lib", "wasm", "ttf", "otf", "woff", "woff2", "eot", "mp3",
        "mp4", "mov", "avi", "wav", "ogg", "webm", "sqlite", "db", "bin", "dat", "pyc", "pyo", "node", "rlib", "rmeta", "car", "nib",
        "psd", "ai", "sketch", "parquet",
    ];
    if let Some((_, ext)) = lower.rsplit_once('.')
        && BINARY.contains(&ext)
    {
        return Some("binary");
    }
    None
}

/// Reject by content. `head` is the first bytes of the file; `text` the decoded whole.
pub(crate) fn skip_by_content(lang: Option<&str>, bytes: &[u8], text: &str, line_count: usize, max_line: usize) -> Option<&'static str> {
    if memchr::memchr(0, &bytes[..bytes.len().min(8192)]).is_some() {
        return Some("binary");
    }
    // Minified bundles and embedded blobs: very long lines.
    if max_line > 20_000 || (bytes.len() > 2048 && bytes.len() / line_count.max(1) > 500) {
        return Some("minified");
    }
    if !matches!(lang, Some("markdown" | "text")) && is_generated(text) {
        return Some("generated");
    }
    None
}

/// Lines at the top of a file where a generator leaves its mark.
const HEADER_LINES: usize = 10;

/// Does the header say a tool wrote this file? Only a statement at the start of a comment
/// sentence counts, never the same words inside ordinary prose: a doc comment that says
/// "bindings are generated by ts-rs" or "filters generated code" belongs to a file a person
/// wrote. A file is generated when a header line
/// - starts with a machine tag (`@generated`, C#'s `<auto-generated>`), or
/// - states it ("Code generated by protoc", "This file was generated by ts-rs",
///   "Automatically generated") and the header also says `do not edit` / `do not modify`, or
///   the statement itself carries the tag ("This file is @generated by prost-build").
fn is_generated(text: &str) -> bool {
    let header: Vec<String> = text.lines().take(HEADER_LINES).map(|l| comment_text(l).to_ascii_lowercase()).collect();
    let hands_off = header.iter().any(|l| l.contains("do not edit") || l.contains("do not modify"));
    header.iter().any(|l| {
        l.starts_with("@generated")
            || l.starts_with("<auto-generated")
            || l.starts_with("<autogenerated")
            || l.split(". ").flat_map(|s| s.split("! ")).any(|s| generated_claim(s.trim_start()).is_some_and(|tagged| tagged || hands_off))
    })
}

/// A line without its indentation and leading comment punctuation.
fn comment_text(line: &str) -> &str {
    const OPENERS: [&str; 15] = ["<!--", "/**", "/*", "//!", "///", "//", "(*", "{-", "*", "#", "--", ";", "%", "\"\"\"", "'''"];
    let mut rest = line.trim_start();
    while let Some(r) = OPENERS.iter().find_map(|o| rest.strip_prefix(o)) {
        rest = r.trim_start();
    }
    rest
}

/// Is this (lowercase) sentence a statement that the file is generated? `Some(true)` when it
/// carries the `@generated` tag.
fn generated_claim(sentence: &str) -> Option<bool> {
    const SUBJECTS: [&str; 9] =
        ["this file is ", "this file was ", "this file has been ", "this code is ", "this code was ", "this is a ", "this is an ", "code ", "file "];
    const ADVERBS: [&str; 4] = ["automatically ", "auto-", "auto ", "auto"];
    const NOUNS: [&str; 3] = [" code", " file", " source"];
    fn strip_one<'a>(s: &'a str, prefixes: &[&str]) -> &'a str {
        prefixes.iter().find_map(|p| s.strip_prefix(p)).unwrap_or(s)
    }
    let s = strip_one(strip_one(sentence, &SUBJECTS), &ADVERBS);
    let tagged = s.starts_with('@');
    let rest = strip_one(s.strip_prefix('@').unwrap_or(s).strip_prefix("generated")?, &NOUNS);
    // What may follow: the end, punctuation, a dash, or the tool ("by protoc", "from x.proto").
    let follows = match rest.strip_prefix(' ') {
        None => rest.is_empty() || rest.starts_with(['.', '!', ':', ',', ';']),
        Some(next) => matches!(next.split(' ').next(), Some("by" | "from" | "with" | "using" | "-" | "--" | "\u{2014}")),
    };
    follows.then_some(tagged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages() {
        assert_eq!(detect_lang("src/a/b.rs"), Some("rust"));
        assert_eq!(detect_lang("web/App.tsx"), Some("typescript"));
        assert_eq!(detect_lang("Makefile"), Some("make"));
        assert_eq!(detect_lang("LICENSE"), None);
    }

    #[test]
    fn skips_noise_by_name() {
        assert_eq!(skip_by_name("Cargo.lock"), Some("lockfile"));
        assert_eq!(skip_by_name("web/dist/app.min.js"), Some("generated"));
        assert_eq!(skip_by_name("logo.png"), Some("binary"));
        assert_eq!(skip_by_name("src/main.rs"), None);
    }

    #[test]
    fn skips_generated_and_minified_content() {
        let t = "// Code generated by protoc. DO NOT EDIT.\npackage x\n";
        assert_eq!(skip_by_content(Some("go"), t.as_bytes(), t, 2, 40), Some("generated"));
        let long = "x".repeat(30_000);
        assert_eq!(skip_by_content(Some("javascript"), long.as_bytes(), &long, 1, 30_000), Some("minified"));
        assert_eq!(skip_by_content(Some("rust"), b"fn main() {}\n", "fn main() {}\n", 1, 12), None);
    }

    fn content_skip(lang: &str, text: &str) -> Option<&'static str> {
        let lines = text.lines().count();
        skip_by_content(Some(lang), text.as_bytes(), text, lines, text.lines().map(str::len).max().unwrap_or(0))
    }

    #[test]
    fn a_header_that_only_mentions_generated_code_is_source() {
        // crates/codly-core/src/model.rs: the contract of the whole app.
        let model = "//! Shared domain and event types. This file is the contract between the Rust core,\n\
                     //! the adapters, and the Svelte UI (TypeScript bindings are generated by ts-rs into\n\
                     //! `src/lib/gen/` when `cargo test -p codly-core` runs).\n//!\n//! Rules:\n\nuse serde::Serialize;\n";
        assert_eq!(content_skip("rust", model), None);
        // This file.
        let lang = "//! Language detection by extension and the \"is this worth indexing\" filters (lockfiles,\n\
                    //! minified bundles, generated code, binaries).\n\nuse std::path::Path;\n";
        assert_eq!(content_skip("rust", lang), None);
        // Written by a tool once, then owned by people: no "do not edit".
        let schema = "# This file is auto-generated from the current state of the database. Instead\n\
                      # of editing this file, please use the migrations feature of Active Record.\n\nActiveRecord::Schema.define do\nend\n";
        assert_eq!(content_skip("ruby", schema), None);
        assert_eq!(content_skip("python", "# Generated by Django 4.2 on 2024-01-01 10:00\n\nfrom django.db import migrations\n"), None);
        // Prose that has both phrases, but not as a statement about the file.
        let prose = "//! Checks generated by the planner are run here.\n//! Do not edit the order: later checks rely on earlier ones.\n\nfn run() {}\n";
        assert_eq!(content_skip("rust", prose), None);
        // The marker far below the header says nothing about the file.
        let late = format!("{}// Code generated by protoc. DO NOT EDIT.\n", "fn a() {}\n".repeat(HEADER_LINES));
        assert_eq!(content_skip("rust", &late), None);
    }

    #[test]
    fn generator_headers_are_skipped() {
        for (lang, text) in [
            ("go", "// Code generated by protoc-gen-go. DO NOT EDIT.\n// versions:\n\npackage pb\n"),
            ("typescript", "// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.\nexport type A = string;\n"),
            ("toml", "# Automatically generated - DO NOT EDIT!\n\n[[permission]]\nidentifier = \"allow-close-office\"\n"),
            ("python", "# -*- coding: utf-8 -*-\n# Generated by the protocol buffer compiler.  DO NOT EDIT!\n# source: a.proto\n"),
            ("dart", "// GENERATED CODE - DO NOT MODIFY BY HAND\n\npart of 'model.dart';\n"),
            ("java", "/*\n * DO NOT EDIT! This file was generated by the build.\n */\npackage a;\n"),
            ("java", "/* AUTO-GENERATED FILE.  DO NOT MODIFY. */\npackage a;\n"),
            ("csharp", "//------------------------------------------------------------------------------\n// <auto-generated>\n//     This code was generated by a tool.\n// </auto-generated>\n"),
            ("rust", "// This file is @generated by prost-build.\npub struct A;\n"),
            ("javascript", "/**\n * @generated SignedSource<<0123456789abcdef>>\n */\nmodule.exports = {};\n"),
            ("toml", "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n"),
        ] {
            assert_eq!(content_skip(lang, text), Some("generated"), "{text}");
        }
    }
}
