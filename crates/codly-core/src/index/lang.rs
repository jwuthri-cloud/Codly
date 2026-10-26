//! Language detection by extension and the "is this worth indexing" filters (lockfiles,
//! minified bundles, generated code, binaries).

use std::path::Path;

/// Directories that hold build output or third-party code in nearly every ecosystem.
/// `.gitignore` already hides most of them in a git checkout; this is the net for repos that
/// do not list them (or are not git repos at all).
pub(crate) fn is_denied_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".tox"
            | ".mypy_cache"
            | ".pytest_cache"
            | ".ruff_cache"
            | ".gradle"
            | ".idea"
            | "Pods"
            | "DerivedData"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | ".turbo"
            | ".parcel-cache"
            | "bower_components"
            | "vendor"
            | "dist"
            | ".build"
            | ".cache"
            | "coverage"
            | ".direnv"
            | ".terraform"
    )
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
    if !matches!(lang, Some("markdown" | "text")) {
        let head = text.as_bytes();
        let head = &head[..head.len().min(1200)];
        let head = String::from_utf8_lossy(head).to_ascii_lowercase();
        for marker in [
            "@generated",
            "do not edit",
            "do not modify",
            "code generated",
            "auto-generated",
            "autogenerated",
            "automatically generated",
            "this file was generated",
            "generated by",
            "generated code",
        ] {
            if head.contains(marker) {
                return Some("generated");
            }
        }
    }
    None
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
}
