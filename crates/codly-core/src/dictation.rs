//! Free Flow dictation: a clip recorded in the webview goes to Groq's Whisper API with the
//! user's own key, and the transcript comes back for the composer draft (never auto-sent).
//!
//! Privacy rules, enforced here rather than left to the UI:
//! - audio leaves the machine only inside `Dictation::transcribe`, which the user triggers by
//!   stopping a recording, and only to the pinned Groq endpoint;
//! - the key lives in the macOS Keychain (`Keychain`), never in `dictation.json`, never in a
//!   log line, and `status()` only says whether one is saved;
//! - a clip is capped at `MAX_AUDIO_BYTES` and the request at `TIMEOUT`.
//!
//! The pure parts (multipart body, response parsing, base64, validation) are unit-tested with
//! a fake `Transport` and `MemorySecrets`; no test touches the network or the Keychain.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Groq's OpenAI-compatible transcription endpoint. The only host audio is ever sent to.
pub const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
pub const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
/// Whisper models Groq serves; anything else is refused before a request is built.
pub const MODELS: [&str; 2] = ["whisper-large-v3-turbo", "whisper-large-v3"];
/// Groq's own upload limit for the free tier.
pub const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Keychain service and account of the Groq key.
const KEYCHAIN_SERVICE: &str = "dev.codly.desktop";
const KEYCHAIN_ACCOUNT: &str = "freeflow-groq-api-key";

// ───────────────────────────── config (not secret) ─────────────────────────────

/// What `dictation.json` in the data dir holds: switches only, never the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DictationConfig {
    pub enabled: bool,
    pub model: String,
}

impl Default for DictationConfig {
    fn default() -> Self {
        DictationConfig { enabled: false, model: DEFAULT_MODEL.into() }
    }
}

/// What the UI sees. There is deliberately no field that can carry the key.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub struct DictationStatus {
    pub enabled: bool,
    pub model: String,
    pub models: Vec<String>,
    /// A key is saved in secure storage ("key saved"); its value never comes back.
    pub key_saved: bool,
    /// False where there is no secure storage to hold a key (anything but macOS today).
    pub secure_storage: bool,
}

impl DictationConfig {
    pub fn load(dir: &Path) -> Self {
        let cfg: DictationConfig =
            std::fs::read(dir.join("dictation.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // A hand-edited file must not smuggle an arbitrary model into the request.
        if MODELS.contains(&cfg.model.as_str()) { cfg } else { DictationConfig { model: DEFAULT_MODEL.into(), ..cfg } }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("dictation.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

// ───────────────────────────── secure storage ─────────────────────────────

/// Where the API key lives. Blocking (the real one spawns `security`).
pub trait SecretStore: Send + Sync + 'static {
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, secret: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
    /// False when `set` can only fail (no secure storage on this platform).
    fn available(&self) -> bool {
        true
    }
}

/// The login Keychain, through `/usr/bin/security`. The key goes in on `security -i`'s stdin
/// (not its argv, which any process of the user can read) and comes out on stdout.
pub struct Keychain;

const SECURITY: &str = "/usr/bin/security";

impl SecretStore for Keychain {
    fn available(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn get(&self) -> Result<Option<String>> {
        if !self.available() {
            return Ok(None);
        }
        let out = std::process::Command::new(SECURITY)
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT, "-w"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .context("could not run the macOS `security` tool")?;
        // Exit 44 = item not found; anything else non-zero (locked keychain, denied) reads as "no key".
        if !out.status.success() {
            return Ok(None);
        }
        let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((!key.is_empty()).then_some(key))
    }

    fn set(&self, secret: &str) -> Result<()> {
        use std::io::Write;
        if !self.available() {
            bail!("secure key storage is only available on macOS in this build");
        }
        let key = validate_key(secret)?; // quoting below is safe because of this
        let mut child = std::process::Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("could not run the macOS `security` tool")?;
        let line = format!("add-generic-password -U -s \"{KEYCHAIN_SERVICE}\" -a \"{KEYCHAIN_ACCOUNT}\" -w \"{key}\"\n");
        child.stdin.take().context("no stdin")?.write_all(line.as_bytes())?;
        // `security -i` exits 0 even when a command fails, so read the key back.
        child.wait()?;
        match self.get()? {
            Some(saved) if saved == key => Ok(()),
            _ => bail!("the Keychain refused to store the key"),
        }
    }

    fn delete(&self) -> Result<()> {
        if !self.available() {
            return Ok(());
        }
        // Not found is fine: the end state is the same.
        let _ = std::process::Command::new(SECURITY)
            .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemorySecrets(std::sync::Mutex<Option<String>>);

impl SecretStore for MemorySecrets {
    fn get(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn set(&self, secret: &str) -> Result<()> {
        *self.0.lock().unwrap() = Some(secret.to_string());
        Ok(())
    }
    fn delete(&self) -> Result<()> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

/// A Groq key is `gsk_` plus letters and digits. Anything with a quote, space or newline is
/// refused, which also keeps it from breaking out of the curl config or `security` line.
pub fn validate_key(raw: &str) -> Result<&str> {
    let key = raw.trim();
    if key.len() < 8 || key.len() > 256 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        bail!("that does not look like a Groq API key (it starts with gsk_)");
    }
    Ok(key)
}

// ───────────────────────────── the request ─────────────────────────────

/// One HTTPS POST. `Debug` hides the bearer token and the audio.
pub struct HttpRequest {
    pub url: String,
    pub bearer: String,
    pub content_type: String,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("bearer", &"<redacted>")
            .field("content_type", &self.content_type)
            .field("body", &format_args!("{} bytes", self.body.len()))
            .finish()
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// How a request reaches Groq. The real one is `CurlTransport`; tests use a fake.
pub trait Transport: Send + Sync + 'static {
    fn post(&self, req: HttpRequest) -> impl std::future::Future<Output = Result<HttpResponse>> + Send;
}

/// File extension Groq sniffs the container from, for the MIME types `MediaRecorder` produces.
pub fn extension_for(mime: &str) -> Result<&'static str> {
    let base = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Ok(match base.as_str() {
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/flac" => "flac",
        other => bail!("unsupported audio type {other:?}"),
    })
}

/// A `multipart/form-data` body: the clip as `file`, plus `model` and `response_format=json`.
pub fn multipart_body(boundary: &str, model: &str, filename: &str, mime: &str, audio: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(audio.len() + 512);
    let mut field = |name: &str, value: &str| {
        b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };
    field("model", model);
    field("response_format", "json");
    field("temperature", "0");
    b.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes(),
    );
    b.extend_from_slice(audio);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    b
}

/// The request for one clip; the boundary is a parameter so tests can pin the bytes.
pub fn build_request(key: &str, model: &str, mime: &str, audio: &[u8], boundary: &str) -> Result<HttpRequest> {
    if audio.is_empty() {
        bail!("the recording is empty");
    }
    if audio.len() > MAX_AUDIO_BYTES {
        bail!("the recording is too long ({} MB; the limit is {} MB)", audio.len() / (1024 * 1024), MAX_AUDIO_BYTES / (1024 * 1024));
    }
    if !MODELS.contains(&model) {
        bail!("unknown model {model:?}");
    }
    let mime = mime.split(';').next().unwrap_or("").trim();
    let ext = extension_for(mime)?;
    Ok(HttpRequest {
        url: ENDPOINT.into(),
        bearer: validate_key(key)?.to_string(),
        content_type: format!("multipart/form-data; boundary={boundary}"),
        body: multipart_body(boundary, model, &format!("dictation.{ext}"), mime, audio),
        timeout: TIMEOUT,
    })
}

/// Groq's answer to the transcript, or a message fit to show the user. Never echoes the body
/// beyond Groq's own `error.message`.
pub fn parse_response(status: u16, body: &[u8]) -> Result<String> {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    if (200..300).contains(&status) {
        return match v["text"].as_str() {
            Some(t) => Ok(t.trim().to_string()),
            None => bail!("Groq's answer had no transcript"),
        };
    }
    let detail = v["error"]["message"].as_str().map(|m| m.chars().take(200).collect::<String>());
    bail!(match status {
        401 | 403 => "Groq rejected the API key. Save a new one in Settings → Free Flow.".to_string(),
        413 => "the recording is too large for Groq".to_string(),
        429 => "Groq says you are sending too many requests; wait a moment and try again".to_string(),
        s => match detail {
            Some(d) => format!("Groq error {s}: {d}"),
            None => format!("Groq error {s}"),
        },
    })
}

/// A recording's bytes from the base64 the webview sends them as (standard alphabet).
pub fn decode_base64(s: &str) -> Result<Vec<u8>> {
    let val = |c: u8| -> Result<u32> {
        Ok(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => bail!("the recording was not valid base64"),
        } as u32)
    };
    let data: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let data = data.strip_suffix(b"==").or_else(|| data.strip_suffix(b"=")).unwrap_or(&data);
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    for chunk in data.chunks(4) {
        if chunk.len() == 1 {
            bail!("the recording was not valid base64");
        }
        let n = chunk.iter().enumerate().try_fold(0u32, |n, (i, &c)| Ok::<_, anyhow::Error>(n | val(c)? << (18 - 6 * i)))?;
        out.extend_from_slice(&n.to_be_bytes()[1..chunk.len()]);
    }
    Ok(out)
}

// ───────────────────────────── the service ─────────────────────────────

/// Everything the commands do: read/switch the config, keep the key, transcribe one clip.
pub struct Dictation<S: SecretStore, T: Transport> {
    dir: PathBuf,
    secrets: Arc<S>,
    transport: T,
}

impl<S: SecretStore, T: Transport> Dictation<S, T> {
    pub fn new(dir: PathBuf, secrets: S, transport: T) -> Self {
        Dictation { dir, secrets: Arc::new(secrets), transport }
    }

    pub fn status(&self) -> DictationStatus {
        let cfg = DictationConfig::load(&self.dir);
        DictationStatus {
            enabled: cfg.enabled,
            model: cfg.model,
            models: MODELS.iter().map(|m| m.to_string()).collect(),
            key_saved: self.secrets.get().ok().flatten().is_some(),
            secure_storage: self.secrets.available(),
        }
    }

    pub fn configure(&self, enabled: Option<bool>, model: Option<String>) -> Result<DictationStatus> {
        let mut cfg = DictationConfig::load(&self.dir);
        if let Some(e) = enabled {
            cfg.enabled = e;
        }
        if let Some(m) = model {
            if !MODELS.contains(&m.as_str()) {
                bail!("unknown model {m:?}");
            }
            cfg.model = m;
        }
        cfg.save(&self.dir)?;
        Ok(self.status())
    }

    pub fn save_key(&self, key: &str) -> Result<DictationStatus> {
        self.secrets.set(validate_key(key)?)?;
        Ok(self.status())
    }

    pub fn clear_key(&self) -> Result<DictationStatus> {
        self.secrets.delete()?;
        Ok(self.status())
    }

    /// Transcribe one recording. Refuses unless Free Flow is on and a key is saved.
    pub async fn transcribe(&self, audio: Vec<u8>, mime: &str) -> Result<String> {
        let cfg = DictationConfig::load(&self.dir);
        if !cfg.enabled {
            bail!("Free Flow is off (Settings → Free Flow)");
        }
        let secrets = self.secrets.clone();
        let key = tokio::task::spawn_blocking(move || secrets.get()).await??;
        let Some(key) = key else { bail!("no Groq API key saved (Settings → Free Flow)") };
        let boundary = format!("codly-{}", uuid::Uuid::new_v4().simple());
        let req = build_request(&key, &cfg.model, mime, &audio, &boundary)?;
        drop(audio);
        let res = self.transport.post(req).await?;
        parse_response(res.status, &res.body)
    }
}

// ───────────────────────────── the real transport ─────────────────────────────

/// POSTs through the system `curl` (always present on macOS), so Codly carries no TLS stack of
/// its own. The key and URL go to curl as a config on stdin, never as arguments (visible in
/// `ps`); the body is a 0600 file in a fresh 0700 directory, removed when the call ends.
pub struct CurlTransport;

impl Transport for CurlTransport {
    async fn post(&self, req: HttpRequest) -> Result<HttpResponse> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        use tokio::io::AsyncWriteExt;

        if req.url != ENDPOINT {
            bail!("refusing to send audio anywhere but Groq");
        }
        let dir = std::env::temp_dir().join(format!("codly-dictation-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let body_path = dir.join("body");
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&body_path)?;
            f.write_all(&req.body)?;
        }
        // Values are quote-free: the key is validated, the URL pinned, the boundary is hex, the
        // path is a uuid under the temp dir.
        let config = format!(
            "url = \"{}\"\nproto = \"=https\"\nsilent\nmax-time = {}\nheader = \"Authorization: Bearer {}\"\nheader = \"Content-Type: {}\"\ndata-binary = \"@{}\"\nwrite-out = \"\\n%{{http_code}}\"\n",
            req.url,
            req.timeout.as_secs(),
            req.bearer,
            req.content_type,
            body_path.display()
        );
        let mut child = tokio::process::Command::new("/usr/bin/curl")
            .args(["-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not run curl")?;
        let mut stdin = child.stdin.take().context("no stdin")?;
        stdin.write_all(config.as_bytes()).await?;
        drop(stdin);
        let out = tokio::time::timeout(req.timeout + Duration::from_secs(5), child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("Groq did not answer in time"))??;
        if !out.status.success() && out.stdout.is_empty() {
            bail!("could not reach Groq (check your connection)");
        }
        // stdout = response body, then "\n<status>" from write-out.
        let split = out.stdout.iter().rposition(|&b| b == b'\n').context("no response from Groq")?;
        let status: u16 = std::str::from_utf8(&out.stdout[split + 1..]).ok().and_then(|s| s.trim().parse().ok()).context("no status from Groq")?;
        Ok(HttpResponse { status, body: out.stdout[..split].to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const KEY: &str = "gsk_testkey1234567890";

    /// Records the request and answers with a canned response.
    struct Fake {
        seen: Mutex<Vec<HttpRequest>>,
        reply: (u16, &'static str),
    }
    impl Fake {
        fn new(status: u16, body: &'static str) -> Self {
            Fake { seen: Mutex::new(vec![]), reply: (status, body) }
        }
    }
    impl Transport for Arc<Fake> {
        async fn post(&self, req: HttpRequest) -> Result<HttpResponse> {
            self.seen.lock().unwrap().push(req);
            Ok(HttpResponse { status: self.reply.0, body: self.reply.1.as_bytes().to_vec() })
        }
    }

    fn service(dir: &Path, fake: &Arc<Fake>) -> Dictation<MemorySecrets, Arc<Fake>> {
        Dictation::new(dir.to_path_buf(), MemorySecrets::default(), fake.clone())
    }

    #[test]
    fn multipart_body_has_the_fields_and_the_audio_verbatim() {
        let b = multipart_body("BND", "whisper-large-v3-turbo", "dictation.webm", "audio/webm", &[0, 1, 2, 255]);
        let text = String::from_utf8_lossy(&b);
        assert!(text.starts_with("--BND\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-large-v3-turbo\r\n"));
        assert!(text.contains("name=\"response_format\"\r\n\r\njson\r\n"));
        assert!(text.contains("name=\"file\"; filename=\"dictation.webm\"\r\nContent-Type: audio/webm\r\n\r\n"));
        assert!(b.windows(4).any(|w| w == [0, 1, 2, 255]));
        assert!(b.ends_with(b"\r\n--BND--\r\n"));
    }

    #[test]
    fn request_goes_to_groq_with_a_bearer_and_hides_it_from_debug() {
        let r = build_request(KEY, DEFAULT_MODEL, "audio/webm;codecs=opus", b"abc", "BND").unwrap();
        assert_eq!(r.url, ENDPOINT);
        assert_eq!(r.content_type, "multipart/form-data; boundary=BND");
        assert!(String::from_utf8_lossy(&r.body).contains("Content-Type: audio/webm\r\n"), "codec parameters are dropped");
        let dbg = format!("{r:?}");
        assert!(!dbg.contains(KEY) && dbg.contains("<redacted>"), "{dbg}");
    }

    #[test]
    fn requests_that_should_not_be_built_are_refused() {
        assert!(build_request(KEY, DEFAULT_MODEL, "audio/webm", b"", "B").is_err(), "empty");
        assert!(build_request(KEY, DEFAULT_MODEL, "audio/webm", &vec![0; MAX_AUDIO_BYTES + 1], "B").is_err(), "too big");
        assert!(build_request(KEY, "gpt-evil\r\n", "audio/webm", b"a", "B").is_err(), "model");
        assert!(build_request(KEY, DEFAULT_MODEL, "text/html", b"a", "B").is_err(), "type");
        assert!(build_request("gsk_\"; x", DEFAULT_MODEL, "audio/webm", b"a", "B").is_err(), "key with a quote");
    }

    #[test]
    fn responses_become_transcripts_or_readable_errors() {
        assert_eq!(parse_response(200, br#"{"text":"  hello there \n"}"#).unwrap(), "hello there");
        assert!(parse_response(200, b"{}").is_err());
        let bad_key = parse_response(401, br#"{"error":{"message":"Invalid API Key"}}"#).unwrap_err().to_string();
        assert!(bad_key.contains("rejected the API key"), "{bad_key}");
        assert!(parse_response(429, b"").unwrap_err().to_string().contains("too many"));
        assert_eq!(parse_response(500, br#"{"error":{"message":"boom"}}"#).unwrap_err().to_string(), "Groq error 500: boom");
        assert_eq!(parse_response(502, b"<html>").unwrap_err().to_string(), "Groq error 502");
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(decode_base64("aGk=").unwrap(), b"hi");
        assert_eq!(decode_base64("aGk").unwrap(), b"hi");
        assert_eq!(decode_base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(decode_base64("/+8A").unwrap(), [0xff, 0xef, 0x00]);
        assert!(decode_base64("a$b=").is_err());
    }

    #[tokio::test]
    async fn transcribe_needs_the_switch_and_a_key_then_returns_the_text() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::new(200, r#"{"text":"add a retry"}"#));
        let d = service(tmp.path(), &fake);

        assert!(d.transcribe(b"x".to_vec(), "audio/webm").await.unwrap_err().to_string().contains("off"));
        d.configure(Some(true), None).unwrap();
        assert!(d.transcribe(b"x".to_vec(), "audio/webm").await.unwrap_err().to_string().contains("no Groq API key"));
        assert!(fake.seen.lock().unwrap().is_empty(), "nothing is sent before the user has opted in");

        d.save_key(KEY).unwrap();
        assert_eq!(d.transcribe(b"audio".to_vec(), "audio/webm").await.unwrap(), "add a retry");
        let seen = fake.seen.lock().unwrap();
        assert_eq!((seen.len(), seen[0].bearer.as_str(), seen[0].url.as_str()), (1, KEY, ENDPOINT));
    }

    #[tokio::test]
    async fn api_errors_surface_and_the_selected_model_is_sent() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::new(401, r#"{"error":{"message":"Invalid API Key"}}"#));
        let d = service(tmp.path(), &fake);
        d.configure(Some(true), Some("whisper-large-v3".into())).unwrap();
        d.save_key(KEY).unwrap();
        assert!(d.transcribe(b"audio".to_vec(), "audio/webm").await.unwrap_err().to_string().contains("rejected"));
        assert!(String::from_utf8_lossy(&fake.seen.lock().unwrap()[0].body).contains("\r\n\r\nwhisper-large-v3\r\n"));
    }

    #[test]
    fn status_never_carries_the_key_and_config_survives_a_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let d = service(tmp.path(), &Arc::new(Fake::new(200, "{}")));
        assert_eq!(d.status(), DictationStatus { enabled: false, model: DEFAULT_MODEL.into(), models: MODELS.map(String::from).to_vec(), key_saved: false, secure_storage: true });

        d.save_key(KEY).unwrap();
        let s = d.configure(Some(true), Some("whisper-large-v3".into())).unwrap();
        assert!(s.key_saved && s.enabled);
        assert!(!serde_json::to_string(&s).unwrap().contains(KEY));
        let on_disk = std::fs::read_to_string(tmp.path().join("dictation.json")).unwrap();
        assert!(!on_disk.contains("gsk_") && on_disk.contains("whisper-large-v3"), "{on_disk}");
        assert!(d.configure(None, Some("evil".into())).is_err());
        assert!(d.save_key("not a key").is_err());

        assert!(!d.clear_key().unwrap().key_saved);
        // A hand-edited model falls back to the default rather than reaching the request.
        std::fs::write(tmp.path().join("dictation.json"), r#"{"enabled":true,"model":"x\ny"}"#).unwrap();
        assert_eq!(DictationConfig::load(tmp.path()), DictationConfig { enabled: true, model: DEFAULT_MODEL.into() });
    }
}
