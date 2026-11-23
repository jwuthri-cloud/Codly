//! Attached images: a picture you paste, drop or pick in the composer reaches the model as a
//! picture, not only as a file path it may never open.
//!
//! The composer saves every attached file (`Core::save_attachment`:
//! `<repo>/.codly/attachments/<id>/<name>`) and names it in the message, one `- <path>` line
//! per file. `AgentCmd::Prompt` stays text: each adapter asks `images` for the attached images
//! of the prompt it is about to send and adds them in its CLI's own form (`claude_content`,
//! `codex_input`, `acp_prompt`). The text is not changed, so the agent can still open the file.
//!
//! What goes as an image:
//! - a file in an attachments folder of Codly, links resolved. No other path is ever read, and
//!   a path named inside an `<untrusted>` block (text from outside, another agent, the repo)
//!   is not an attachment;
//! - PNG, JPEG, GIF or WebP, told from the first bytes of the file, never from its name. An
//!   ACP agent gets no GIF as an image (`ACP_MEDIA_TYPES`);
//! - at most `MAX_IMAGE_BYTES` each and `MAX_PROMPT_BYTES` in one prompt. An image over a
//!   limit stays a file path, and the user is told (`Attached::note`).
//!
//! Any other attached file, and one that cannot be read, stays a file path as before. The
//! heading the composer writes over the list says where the files are and gives no order to
//! read them, so a model that already has the image in the message does not open it again.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

/// The largest image sent as an image. Its base64 form stays under the 5 MB that the
/// Anthropic API takes for one image.
const MAX_IMAGE_BYTES: u64 = 3_500_000;

/// The images of one prompt together: they travel in one line on the CLI's stdin.
const MAX_PROMPT_BYTES: u64 = 10_000_000;

/// The first bytes that tell the four formats apart (WebP needs 12).
const HEAD_BYTES: u64 = 12;

/// The formats an ACP agent gets as an image block. `promptCapabilities.image` is one flag
/// with no list of formats, and the model behind the agent is not known, so these are the
/// formats every vision provider takes: the Gemini API lists no GIF for image input, and a
/// block the provider refuses fails the turn. A GIF stays a file path for these agents, as
/// before (docs/research/cli-others-acp.md §1.5).
const ACP_MEDIA_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// An attached image, read and ready to go to a model.
pub(super) struct Image {
    /// Where the file is, links resolved.
    path: PathBuf,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    media_type: &'static str,
    bytes: Vec<u8>,
}

/// The attached images of one prompt.
#[derive(Default)]
pub(super) struct Attached {
    pub images: Vec<Image>,
    /// For the user: the images that are over a limit, so they went as a file path only.
    pub note: Option<String>,
}

/// The images among the files `prompt` names as attached, in the order it names them.
pub(super) async fn images(prompt: &str) -> Attached {
    images_of(prompt, |_| true).await
}

/// `images` for an ACP agent that takes images: only the formats of `ACP_MEDIA_TYPES`. An
/// image of another format is not counted in the limits, and there is no note about it.
pub(super) async fn acp_images(prompt: &str) -> Attached {
    images_of(prompt, |media_type| ACP_MEDIA_TYPES.contains(&media_type)).await
}

/// The attached images of `prompt` whose media type `takes` accepts.
async fn images_of(prompt: &str, takes: fn(&str) -> bool) -> Attached {
    let mut attached = Attached::default();
    let mut left_out = Vec::new();
    let mut room = MAX_PROMPT_BYTES;
    let as_a_path = "so the agent gets it as a file path, not as an image.";
    for named in named_attachments(prompt) {
        let name = named.file_name().unwrap_or_default().to_string_lossy();
        match load(named, room, takes).await {
            Loaded::Image(image) => {
                room = room.saturating_sub(image.bytes.len() as u64);
                attached.images.push(image);
            }
            Loaded::NotAnImage => {}
            Loaded::TooBig => left_out.push(format!("{name} is over {} MB, {as_a_path}", megabytes(MAX_IMAGE_BYTES))),
            Loaded::NoRoom => {
                left_out.push(format!("{name} takes the images of this message over {} MB, {as_a_path}", megabytes(MAX_PROMPT_BYTES)))
            }
        }
    }
    if !left_out.is_empty() {
        attached.note = Some(left_out.join(" "));
    }
    attached
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / 1e6
}

/// The attachment paths `prompt` names, each once: lines of the form `- <path>` (the
/// composer's list) outside `<untrusted>` blocks. Codly neutralizes these tags inside quoted
/// text (`prompts::untrusted`), so a block cannot be closed from within.
fn named_attachments(prompt: &str) -> Vec<&Path> {
    let mut paths: Vec<&Path> = Vec::new();
    let mut quoted = false;
    for line in prompt.lines() {
        match (line.rfind("<untrusted"), line.rfind("</untrusted>")) {
            (Some(open), Some(close)) => quoted = open > close,
            (Some(_), None) => quoted = true,
            (None, Some(_)) => quoted = false,
            (None, None) if !quoted => {
                let named = line.trim().strip_prefix("- ").map(Path::new).filter(|p| is_attachment(p) && !paths.contains(p));
                paths.extend(named);
            }
            (None, None) => {}
        }
    }
    paths
}

/// Is `path` a file of an attachments folder: absolute, and ending in
/// `.codly/attachments/<id>/<name>`?
fn is_attachment(path: &Path) -> bool {
    let mut last = path.components().rev();
    let (name, id, folder, root) = (last.next(), last.next(), last.next(), last.next());
    path.is_absolute()
        && matches!(name, Some(Component::Normal(_)))
        && matches!(id, Some(Component::Normal(_)))
        && matches!(folder, Some(Component::Normal(f)) if f == "attachments")
        && matches!(root, Some(Component::Normal(r)) if r == ".codly")
}

enum Loaded {
    Image(Image),
    /// Another kind of file, an image format the agent does not get, or a file that cannot
    /// be read: it stays a file path, as before.
    NotAnImage,
    /// Over `MAX_IMAGE_BYTES`.
    TooBig,
    /// It does not fit in what is left of `MAX_PROMPT_BYTES`.
    NoRoom,
}

/// Read the attachment `named` when it is an image of at most `room` bytes, of a media type
/// `takes` accepts.
async fn load(named: &Path, room: u64, takes: fn(&str) -> bool) -> Loaded {
    // Links resolved: a link in the folder that leads to a file elsewhere is not an attachment.
    let Ok(path) = tokio::fs::canonicalize(named).await else { return Loaded::NotAnImage };
    if !is_attachment(&path) {
        return Loaded::NotAnImage;
    }
    // A regular file only: opening a pipe would wait without end.
    let Ok(meta) = tokio::fs::metadata(&path).await else { return Loaded::NotAnImage };
    if !meta.is_file() {
        return Loaded::NotAnImage;
    }
    let Ok(mut file) = tokio::fs::File::open(&path).await else { return Loaded::NotAnImage };
    let mut bytes = Vec::new();
    if (&mut file).take(HEAD_BYTES).read_to_end(&mut bytes).await.is_err() {
        return Loaded::NotAnImage;
    }
    let Some(media_type) = media_type(&bytes).filter(|m| takes(m)) else { return Loaded::NotAnImage };
    if meta.len() > MAX_IMAGE_BYTES {
        return Loaded::TooBig;
    }
    if meta.len() > room {
        return Loaded::NoRoom;
    }
    // The size was checked above; `take` holds it for a file that grew since.
    if file.take(meta.len().saturating_sub(bytes.len() as u64)).read_to_end(&mut bytes).await.is_err() {
        return Loaded::NotAnImage;
    }
    Loaded::Image(Image { path, media_type, bytes })
}

/// The media type of an image, from the first bytes of its file.
fn media_type(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Claude: the `content` of a stream-json user message. With images it is a list of base64
/// image blocks and then the text, as recorded from Claude Code 2.1.311
/// (docs/research/cli-claude-stream-json-2.1.311.md §3, fixture `image-input`); without, the
/// text alone.
pub(super) fn claude_content(text: &str, images: &[Image]) -> Value {
    if images.is_empty() {
        return json!(text);
    }
    let mut blocks: Vec<Value> = images
        .iter()
        .map(|i| json!({ "type": "image", "source": { "type": "base64", "media_type": i.media_type, "data": base64(&i.bytes) } }))
        .collect();
    blocks.push(json!({ "type": "text", "text": text }));
    Value::Array(blocks)
}

/// Codex: the `input` of `turn/start` and `turn/steer`. An image is a `localImage` item with
/// its path (`UserInput`, from codex-cli 0.166.0 `app-server generate-ts`): Codex reads the
/// file itself.
pub(super) fn codex_input(text: &str, images: &[Image]) -> Value {
    let mut items = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
    items.extend(images.iter().map(|i| json!({ "type": "localImage", "path": i.path.to_string_lossy() })));
    Value::Array(items)
}

/// ACP: the `prompt` of `session/prompt`. An image is an image content block with base64
/// `data` (docs/research/cli-others-acp.md §1.5). Only for an agent whose
/// `promptCapabilities.image` is true: the caller passes the images of `acp_images` to it and
/// no image to any other.
pub(super) fn acp_prompt(text: &str, images: &[Image]) -> Value {
    let mut blocks = vec![json!({ "type": "text", "text": text })];
    blocks.extend(images.iter().map(|i| json!({ "type": "image", "mimeType": i.media_type, "data": base64(&i.bytes) })));
    Value::Array(blocks)
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16 | u32::from(*chunk.get(1).unwrap_or(&0)) << 8 | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Test files for the adapters: an attachments folder with images in it, and the message the
/// composer writes for them.
#[cfg(test)]
pub(super) mod testing {
    use std::path::{Path, PathBuf};

    /// The first bytes of every PNG: a file that starts with them reads as one.
    pub const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n";

    /// Save `bytes` as the attachment `name` of the repo `repo`; its path, links resolved.
    pub fn attach(repo: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let dir = repo.join(".codly/attachments").join(format!("id-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
        dir.join(name).canonicalize().unwrap()
    }

    /// `PNG_HEAD` and zeros: a file of `len` bytes that reads as a PNG.
    pub fn png_of(len: usize) -> Vec<u8> {
        let mut bytes = PNG_HEAD.to_vec();
        bytes.resize(len, 0);
        bytes
    }

    /// What the composer sends for `text` with `files` attached (`messageWithFiles` in
    /// src/home/attached.ts).
    pub fn message(text: &str, files: &[&Path]) -> String {
        let list: Vec<String> = files.iter().map(|f| format!("- {}", f.display())).collect();
        format!("{text}\n\nAttached files, saved at these paths (an image may already be in this message):\n{}", list.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{PNG_HEAD, attach, message, png_of};
    use super::*;

    /// The first message Codly's recording of Claude Code 2.1.311 sent with an image.
    fn recorded_image_message() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/fixtures/claude-2.1.311/image-input.ndjson");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find_map(|l| l.get("_sent").cloned()).expect("a sent line")
    }

    #[test]
    fn base64_matches_the_standard_test_vectors() {
        // RFC 4648 §10
        for (plain, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
        assert_eq!(base64(&[0xff, 0xef, 0x00]), "/+8A");
    }

    #[test]
    fn the_four_formats_are_told_from_the_first_bytes() {
        assert_eq!(media_type(PNG_HEAD), Some("image/png"));
        assert_eq!(media_type(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(media_type(b"GIF89a.."), Some("image/gif"));
        assert_eq!(media_type(b"GIF87a.."), Some("image/gif"));
        assert_eq!(media_type(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        // other RIFF files (a WAV), other pictures (a BMP), text, a cut file
        assert_eq!(media_type(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(media_type(b"BM\x36\0\0\0"), None);
        assert_eq!(media_type(b"<svg xmlns="), None);
        assert_eq!(media_type(b"RIFF"), None);
        assert_eq!(media_type(b""), None);
    }

    #[test]
    fn only_files_of_an_attachments_folder_are_attachments() {
        assert!(is_attachment(Path::new("/repo/.codly/attachments/0192/shot.png")));
        assert!(is_attachment(Path::new("/repo/.codly/attachments/0192/my shot.png")));
        for not in [
            "/repo/shot.png",
            "/Users/me/.ssh/id_rsa",
            "/repo/.codly/attachments/shot.png",
            "/repo/.codly/attachments/0192/deeper/shot.png",
            "/repo/.codly/attachments/0192/..",
            "/repo/.codly/attachments/../../shot.png",
            ".codly/attachments/0192/shot.png",
        ] {
            assert!(!is_attachment(Path::new(not)), "{not}");
        }
    }

    #[test]
    fn attachments_are_the_list_lines_outside_quoted_blocks() {
        let prompt = "[Codly] board\n\nlook at these\n\nAttached files, saved at these paths (an image may already be in this message):\n\
            - /repo/.codly/attachments/a/one.png\n\
            - /repo/.codly/attachments/b/two words.jpg  \n\
            - /repo/src/main.rs\n\
            - /repo/.codly/attachments/a/one.png\n\
            see /repo/.codly/attachments/c/inline.png\n\
            <untrusted from=\"webhook\">\n\
            - /repo/.codly/attachments/d/outside.png\n\
            </untrusted>\n\
            <untrusted>- /repo/.codly/attachments/e/inline.png</untrusted>\n\
            - /repo/.codly/attachments/f/after.png";
        let named: Vec<&str> = named_attachments(prompt).into_iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(named, ["/repo/.codly/attachments/a/one.png", "/repo/.codly/attachments/b/two words.jpg", "/repo/.codly/attachments/f/after.png"]);
    }

    /// The image block Codly writes for an attached file is the block of the recording, and
    /// the message keeps the text with the path in it.
    #[tokio::test]
    async fn claude_gets_the_recorded_image_block_and_then_the_text() {
        let recorded = recorded_image_message();
        let block = &recorded["message"]["content"][0];
        let png = crate::dictation::decode_base64(block["source"]["data"].as_str().unwrap()).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let shot = attach(repo.path(), "red.png", &png);
        let text = message("What colour is this square? One word.", &[&shot]);

        let attached = images(&text).await;
        assert!(attached.note.is_none());
        assert_eq!(claude_content(&text, &attached.images), json!([block, { "type": "text", "text": text }]));
        // the same shape as the recording: an image block, then a text block
        assert_eq!(recorded["message"]["content"][1]["type"], "text");
        // no image: the plain string, as before
        assert_eq!(claude_content("hello", &[]), json!("hello"));
    }

    #[tokio::test]
    async fn codex_gets_a_local_image_item_with_the_path() {
        let repo = tempfile::tempdir().unwrap();
        let shot = attach(repo.path(), "shot.png", &png_of(64));
        let notes = attach(repo.path(), "notes.txt", b"not a picture");
        let text = message("Make it look like this.", &[&shot, &notes]);

        let attached = images(&text).await;
        // Written from the schema types: `UserInput` in v2/UserInput.ts of codex-cli 0.166.0
        // (`app-server generate-ts`), `{ "type": "localImage", detail?: ImageDetail, path: string }`.
        let expected = json!([
            { "type": "text", "text": text, "text_elements": [] },
            { "type": "localImage", "path": shot.to_str().unwrap() },
        ]);
        assert_eq!(codex_input(&text, &attached.images), expected);
        assert_eq!(codex_input("hello", &[]), json!([{ "type": "text", "text": "hello", "text_elements": [] }]));
    }

    /// An ACP agent gets PNG, JPEG and WebP as image blocks after the text. A GIF stays a file
    /// path for it without a word, of any size, where Claude and Codex get the GIF as an image.
    #[tokio::test]
    async fn an_acp_agent_gets_an_image_block_after_the_text_but_no_gif() {
        let repo = tempfile::tempdir().unwrap();
        let jpeg: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 0];
        let webp: &[u8] = b"RIFF\x10\0\0\0WEBPVP8 ";
        let gif: &[u8] = b"GIF89a\x01\0\x01\0";
        let mut long_gif = gif.to_vec();
        long_gif.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        let files = [
            attach(repo.path(), "shot.png", &png_of(16)),
            attach(repo.path(), "anim.gif", gif),
            attach(repo.path(), "photo.jpg", jpeg),
            attach(repo.path(), "long.gif", &long_gif),
            attach(repo.path(), "art.webp", webp),
        ];
        let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        let text = message("What is on these?", &paths);

        let attached = acp_images(&text).await;
        assert!(attached.note.is_none(), "{:?}", attached.note);
        let expected = json!([
            { "type": "text", "text": text },
            { "type": "image", "mimeType": "image/png", "data": base64(&png_of(16)) },
            { "type": "image", "mimeType": "image/jpeg", "data": base64(jpeg) },
            { "type": "image", "mimeType": "image/webp", "data": base64(webp) },
        ]);
        assert_eq!(acp_prompt(&text, &attached.images), expected);
        assert_eq!(acp_prompt("hello", &[]), json!([{ "type": "text", "text": "hello" }]));

        let all = images(&text).await;
        let sent: Vec<&str> = all.images.iter().map(|i| i.media_type).collect();
        assert_eq!(sent, ["image/png", "image/gif", "image/jpeg", "image/webp"]);
        assert_eq!(all.note.as_deref(), Some("long.gif is over 3.5 MB, so the agent gets it as a file path, not as an image."));
    }

    /// The type comes from the file, not from its name; a file that is no image, a file that
    /// is gone and a file outside the attachments folders stay a path.
    #[tokio::test]
    async fn only_real_images_of_the_attachments_folder_are_sent() {
        let repo = tempfile::tempdir().unwrap();
        let jpeg = attach(repo.path(), "photo.png", &[0xff, 0xd8, 0xff, 0xe0, 0, 0]);
        let fake = attach(repo.path(), "fake.png", b"hello world");
        let gone = repo.path().join(".codly/attachments/x/gone.png");
        let elsewhere = repo.path().join("logo.png");
        std::fs::write(&elsewhere, png_of(32)).unwrap();
        let text = message("see", &[&jpeg, &fake, &gone, &elsewhere]);

        let attached = images(&text).await;
        assert!(attached.note.is_none(), "{:?}", attached.note);
        let sent: Vec<(&Path, &str)> = attached.images.iter().map(|i| (i.path.as_path(), i.media_type)).collect();
        assert_eq!(sent, [(jpeg.as_path(), "image/jpeg")]);
    }

    /// A link in an attachments folder does not make a file elsewhere an attachment.
    #[tokio::test]
    async fn a_link_to_a_file_elsewhere_is_not_read() {
        let repo = tempfile::tempdir().unwrap();
        let private = repo.path().join("private.png");
        std::fs::write(&private, png_of(32)).unwrap();
        let dir = repo.path().join(".codly/attachments/a1");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&private, dir.join("link.png")).unwrap();
        std::os::unix::fs::symlink(repo.path(), repo.path().join(".codly/attachments/a2")).unwrap();
        let text = message("see", &[&dir.join("link.png"), &repo.path().join(".codly/attachments/a2/private.png")]);

        let attached = images(&text).await;
        assert!(attached.images.is_empty() && attached.note.is_none());
    }

    /// An image over a limit stays a file path, and the user is told which one and why.
    #[tokio::test]
    async fn an_image_over_a_limit_is_left_out_with_a_note() {
        let repo = tempfile::tempdir().unwrap();
        let big = attach(repo.path(), "big.png", &png_of(MAX_IMAGE_BYTES as usize + 1));
        let most = MAX_IMAGE_BYTES as usize;
        let fits = [attach(repo.path(), "a.png", &png_of(most)), attach(repo.path(), "b.png", &png_of(most))];
        let over = attach(repo.path(), "c.png", &png_of(most));
        let small = attach(repo.path(), "d.png", &png_of(64));
        // a big file that is no image is no image: nothing to say about it
        let zip = attach(repo.path(), "logs.zip", &vec![b'P'; MAX_IMAGE_BYTES as usize + 1]);
        let text = message("see", &[&big, &fits[0], &fits[1], &over, &small, &zip]);

        let attached = images(&text).await;
        let sent: Vec<&Path> = attached.images.iter().map(|i| i.path.as_path()).collect();
        assert_eq!(sent, [fits[0].as_path(), fits[1].as_path(), small.as_path()], "a small image still fits after one that did not");
        assert_eq!(
            attached.note.as_deref(),
            Some(
                "big.png is over 3.5 MB, so the agent gets it as a file path, not as an image. \
                 c.png takes the images of this message over 10 MB, so the agent gets it as a file path, not as an image."
            )
        );
    }
}
