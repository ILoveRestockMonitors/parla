//! Local status, settings and recovery dashboard.
//! Dependency-free HTTP/1.1 server bound to 127.0.0.1 ONLY; serves the
//! embedded single-page UI plus a freshly-probed /api/status JSON.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BIND_ADDR: &str = "127.0.0.1:9393";
const HTML: &str = include_str!("../assets/dashboard.html");
const ASR_PORT: u16 = 9292;
const FORMATTER_PORT: u16 = 11434;
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 128 * 1024;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static SETTINGS_WRITE: Mutex<()> = Mutex::new(());

pub fn open_in_browser() {
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(not(target_os = "macos"))]
        let opener = "xdg-open";
        if let Err(e) = std::process::Command::new(opener)
            .arg("http://127.0.0.1:9393/")
            .spawn()
        {
            eprintln!("[parla] browser unavailable ({e}); open http://127.0.0.1:9393/ manually.");
        }
    }
    #[cfg(windows)]
    unsafe {
        use windows::core::{w, PCWSTR};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        // Fixed localhost URL only. No downloaded content or settings become commands.
        let _ = ShellExecuteW(
            HWND(0),
            w!("open"),
            w!("http://127.0.0.1:9393/"),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Start the server on a detached thread; returns immediately.
/// Port-busy is surfaced to the caller (run_live decides whether it's fatal).
pub fn spawn() -> Result<(), String> {
    let listener =
        TcpListener::bind(BIND_ADDR).map_err(|e| format!("dashboard bind {BIND_ADDR}: {e}"))?;
    let started = Instant::now();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    if ACTIVE
                        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                            (n < 4).then_some(n + 1)
                        })
                        .is_err()
                    {
                        continue;
                    }
                    if std::thread::Builder::new()
                        .name("parla-dash-conn".into())
                        .spawn(move || {
                            handle(s, started);
                            ACTIVE.fetch_sub(1, Ordering::SeqCst);
                        })
                        .is_err()
                    {
                        ACTIVE.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(e) => eprintln!("[parla] dashboard accept: {e}"),
            }
        }
    });
    println!("[parla] dashboard on http://{BIND_ADDR}");
    Ok(())
}

fn handle(mut stream: TcpStream, started: Instant) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                &json!({"error":e}).to_string(),
            );
            return;
        }
    };
    let host = header(&req, "host").unwrap_or_default();
    if host != "127.0.0.1:9393" && host != "localhost:9393" {
        write_json(
            &mut stream,
            "400 Bad Request",
            "{\"error\":\"invalid host\"}",
        );
        return;
    }
    if let Some(origin) = header(&req, "origin") {
        if origin != "http://127.0.0.1:9393" && origin != "http://localhost:9393" {
            write_json(
                &mut stream,
                "403 Forbidden",
                "{\"error\":\"invalid origin\"}",
            );
            return;
        }
    }
    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let method = req.split_whitespace().next().unwrap_or("");
    let is_post = method == "POST";
    let control = header(&req, "x-parla-control").as_deref() == Some("1");
    if is_post
        && header(&req, "content-type")
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            != "application/json"
    {
        write_json(
            &mut stream,
            "415 Unsupported Media Type",
            "{\"error\":\"JSON content type required\"}",
        );
        return;
    }
    if !is_post && method != "GET" {
        write_json(
            &mut stream,
            "405 Method Not Allowed",
            "{\"error\":\"method unsupported\"}",
        );
        return;
    }

    // POST /api/backend {"backend":"whisper"|"parakeet"}: persist to
    // settings.json AND flip the live runtime selector (next dictation uses
    // the new engine; no restart). GET /api/status carries backend info.
    if is_post && path == "/api/command" {
        if !control {
            write_json(
                &mut stream,
                "403 Forbidden",
                "{\"error\":\"control header required\"}",
            );
            return;
        }
        let body = req
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.trim())
            .unwrap_or("");
        let result = serde_json::from_str::<Value>(body).ok().and_then(|v| {
            v.get("command")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        });
        let parsed = serde_json::from_str::<Value>(body).ok();
        let ok = match result.as_deref() {
            Some("toggle") => crate::hotkey::request_toggle(),
            Some("stop") => crate::runtime::enqueue(crate::runtime::Command::Stop),
            Some("cancel") => crate::runtime::enqueue(crate::runtime::Command::Cancel),
            Some("retry") => crate::runtime::enqueue(crate::runtime::Command::Retry),
            Some("purge") => crate::runtime::enqueue(crate::runtime::Command::Purge),
            Some("restore") => crate::runtime::enqueue(crate::runtime::Command::Restore),
            Some("learn") => parsed
                .as_ref()
                .and_then(|v| {
                    Some(crate::runtime::enqueue(crate::runtime::Command::Learn {
                        alias: v.get("alias")?.as_str()?.to_string(),
                        canonical: v.get("canonical")?.as_str()?.to_string(),
                    }))
                })
                .unwrap_or(false),
            _ => false,
        };
        let body = if ok {
            "{\"ok\":true}".to_string()
        } else {
            "{\"error\":\"invalid or full command queue\"}".to_string()
        };
        let status = if ok { "200 OK" } else { "400 Bad Request" };
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
        return;
    }
    if is_post && path == "/api/settings" {
        if !control {
            write_json(
                &mut stream,
                "403 Forbidden",
                "{\"error\":\"control header required\"}",
            );
            return;
        }
        let body = req
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.trim())
            .unwrap_or("");
        match update_settings(body) {
            Ok(v) => write_json(&mut stream, "200 OK", &v),
            Err(e) => write_json(
                &mut stream,
                "400 Bad Request",
                &format!(
                    "{{\"error\":{}}}",
                    serde_json::to_string(&e).unwrap_or_else(|_| "\"settings error\"".into())
                ),
            ),
        }
        return;
    }
    if is_post && path == "/api/backend" {
        if !control {
            write_json(
                &mut stream,
                "403 Forbidden",
                "{\"error\":\"control header required\"}",
            );
            return;
        }
        let (status, ctype, body) = match handle_backend_flip(&req) {
            Ok(msg) => ("200 OK", "application/json", msg),
            Err(e) => ("400 Bad Request", "application/json", e),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\
             \r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
        return;
    }

    let (status, ctype, body) = match path.as_str() {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", HTML.to_string()),
        "/api/status" => ("200 OK", "application/json", status_json(started)),
        "/api/setup" => (
            "200 OK",
            "application/json",
            crate::setup::inspect(&crate::store::settings::Settings::load()).to_string(),
        ),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_string(),
        ),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn write_json(stream: &mut TcpStream, status: &str, body: &str) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn update_settings(body: &str) -> Result<String, String> {
    let _guard = SETTINGS_WRITE
        .lock()
        .map_err(|_| "settings lock poisoned")?;
    let patch: Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    let path = crate::platform::settings_path();
    std::fs::create_dir_all(crate::platform::data_dir()).map_err(|e| e.to_string())?;
    let value = read_writable_settings(&path)?;
    let (value, s) = merge_settings(value, patch)?;
    crate::store::settings::atomic_write(
        &path,
        &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )?;
    crate::runtime::update(|r| r.settings = s.clone());
    crate::asr::set_backend(s.asr_backend_kind());
    Ok(json!({"ok":true,"settings":s}).to_string())
}

fn read_writable_settings(path: &std::path::Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("Settings unreadable; original preserved: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(defaults_json()),
        Err(e) => Err(format!("Settings read: {e}")),
    }
}

fn merge_settings(
    mut value: Value,
    patch: Value,
) -> Result<(Value, crate::store::settings::Settings), String> {
    if value
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(2)
        > 2
    {
        return Err("Settings schema is newer than this build; original preserved".into());
    }
    let obj = value.as_object_mut().ok_or("settings are not an object")?;
    let allowed = [
        "cleanup_mode",
        "stutter_correction",
        "history_mode",
        "retain_audio_for_retry",
        "retry_audio_seconds",
        "max_recording_seconds",
        "chimes_enabled",
        "hud_enabled",
        "asr_backend",
    ];
    for (k, v) in patch
        .as_object()
        .ok_or("settings patch must be an object")?
    {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("Setting is not editable here: {k}"));
        }
        obj.insert(k.clone(), v.clone());
    }
    let mut s: crate::store::settings::Settings =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid setting: {e}"))?;
    s.normalize();
    // Preserve future/third-party keys while storing normalized typed settings.
    let normalized = serde_json::to_value(&s).map_err(|e| e.to_string())?;
    value
        .as_object_mut()
        .unwrap()
        .extend(normalized.as_object().unwrap().clone());
    Ok((value, s))
}

fn header(req: &str, name: &str) -> Option<String> {
    req.split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.eq_ignore_ascii_case(name).then_some(v.trim().to_string())
        })
}
fn read_request(stream: &mut impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut one = [0u8; 1024];
    let end;
    loop {
        let n = stream.read(&mut one).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("truncated request".into());
        }
        bytes.extend_from_slice(&one[..n]);
        if bytes.len() > MAX_HEADER + MAX_BODY {
            return Err("request too large".into());
        }
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            end = p + 4;
            if end > MAX_HEADER {
                return Err("headers too large".into());
            }
            break;
        }
        if bytes.len() > MAX_HEADER {
            return Err("headers too large".into());
        }
    }
    let head = String::from_utf8(bytes[..end].to_vec()).map_err(|_| "headers not utf8")?;
    for name in [
        "host",
        "origin",
        "content-length",
        "content-type",
        "x-parla-control",
    ] {
        if head
            .split("\r\n")
            .skip(1)
            .filter(|line| {
                line.split_once(':')
                    .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
            })
            .count()
            > 1
        {
            return Err(format!("duplicate {name}"));
        }
    }
    if header(&head, "transfer-encoding").is_some() {
        return Err("transfer encoding unsupported".into());
    }
    let lengths: Vec<usize> = head
        .split("\r\n")
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("invalid content length")?;
    if lengths.windows(2).any(|w| w[0] != w[1]) {
        return Err("duplicate content length mismatch".into());
    }
    let want = *lengths.first().unwrap_or(&0);
    if want > MAX_BODY {
        return Err("body too large".into());
    }
    while bytes.len() - end < want {
        let n = stream.read(&mut one).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("truncated body".into());
        }
        bytes.extend_from_slice(&one[..n]);
    }
    if bytes.len() - end != want {
        return Err("body length mismatch".into());
    }
    Ok(String::from_utf8(bytes).map_err(|_| "request not utf8")?)
}

/// Flip the ASR backend: validate body, persist, flip runtime selector.
fn handle_backend_flip(req: &str) -> Result<String, String> {
    let body = req
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or_default();
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| {
        "{\"error\":\"body must be JSON {\\\"backend\\\":\\\"whisper\\\"|\\\"parakeet\\\"}\"}"
            .to_string()
    })?;
    let want = v
        .get("backend")
        .and_then(Value::as_str)
        .ok_or("{\"error\":\"missing backend field\"}".to_string())?
        .to_ascii_lowercase();
    let kind = match want.as_str() {
        "whisper" => crate::store::settings::AsrBackend::Whisper,
        "parakeet" => crate::store::settings::AsrBackend::Parakeet,
        _ => return Err("{\"error\":\"backend must be whisper or parakeet\"}".to_string()),
    };

    // Parakeet gate: refuse the flip if the weights were never dropped in.
    if kind == crate::store::settings::AsrBackend::Parakeet {
        let c = crate::asr::parakeet_local::ParakeetClient::from_settings(
            &crate::store::settings::Settings::load(),
        );
        if !c.weights_present() {
            return Err(
                json!({"error":format!("Parakeet weights missing in {}",c.model_dir)}).to_string(),
            );
        }
    }

    persist_backend(&want)?;
    crate::asr::set_backend(kind);
    Ok(format!("{{\"ok\":true,\"backend\":\"{want}\"}}"))
}

/// Read-modify-write settings.json, creating it from typed defaults if absent.
/// Corrupt file => refuse (never clobber user data we could not parse).
fn persist_backend(backend: &str) -> Result<(), String> {
    let _guard = SETTINGS_WRITE
        .lock()
        .map_err(|_| "settings lock poisoned")?;
    let path = crate::platform::settings_path();
    std::fs::create_dir_all(crate::platform::data_dir()).map_err(|e| e.to_string())?;
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(txt) => serde_json::from_str(&txt).map_err(|e| {
            format!("settings.json unreadable ({e}); fix it before switching backends")
        })?,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::to_value(crate::store::settings::Settings::default())
                .map_err(|e| e.to_string())?
        }
        Err(e) => return Err(format!("settings read: {e}")),
    };
    if root
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(2)
        > 2
    {
        return Err("settings schema is newer than this build".into());
    }
    let obj = root.as_object_mut().ok_or("Settings are not an object")?;
    obj.insert("asr_backend".into(), Value::String(backend.to_string()));
    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    crate::store::settings::atomic_write(std::path::Path::new(&path), &bytes)
        .map_err(|e| format!("settings write: {e}"))
}

fn status_json(started: Instant) -> String {
    // Probes run in parallel so worst-case response stays under ~1 s even
    // when both services are dead (each probe is individually time-bounded).
    let active_backend = crate::store::settings::Settings::load().asr_backend_kind();
    let active_port = if active_backend == crate::store::settings::AsrBackend::Parakeet {
        9293
    } else {
        ASR_PORT
    };
    let asr_h = std::thread::spawn(move || probe_tcp(active_port));

    // Settings are read per request so external edits show up live; ANY
    // failure falls back to compiled defaults and is flagged to the UI.
    let (settings, settings_error) = load_settings();
    let formatter_port = settings
        .get("formatter_port")
        .and_then(Value::as_u64)
        .unwrap_or(FORMATTER_PORT as u64) as u16;
    let fmt_h = std::thread::spawn(move || probe_tcp(formatter_port));

    let model = settings
        .get("formatter_model")
        .and_then(Value::as_str)
        .unwrap_or("qwen3.5:9b")
        .to_string();
    let loaded = {
        let model = model.clone();
        std::thread::spawn(move || formatter_pinned(&model, formatter_port))
    };

    let (_, asr_ms) = asr_h.join().unwrap_or((false, 0));
    let (fmt_up, fmt_ms) = fmt_h.join().unwrap_or((false, 0));
    let loaded = loaded.join().unwrap_or(false);

    // Backend selector + engine-specific liveness (parakeet = /health probe).
    let settings_typed = crate::store::settings::Settings::load();
    let parakeet_client =
        crate::asr::parakeet_local::ParakeetClient::from_settings(&settings_typed);
    let parakeet_up = parakeet_client.alive();
    let whisper_up = crate::asr::whisper_ready(9292);
    let active_up = if active_backend == crate::store::settings::AsrBackend::Parakeet {
        parakeet_up
    } else {
        whisper_up
    };
    let parakeet_weights = parakeet_client.weights_present();

    let runtime = crate::runtime::snapshot();
    json!({
        "asr": {"alive": active_up, "connection_probe_ms": asr_ms, "port": active_port},
        "asr_backend": {
            "active": crate::asr::backend_name(active_backend),
            "whisper_model_verification": crate::asr::whisper_model_verification(),
            "whisper_alive": whisper_up,
            "parakeet_alive": parakeet_up,
            "parakeet_weights_present": parakeet_weights,
        },
        "formatter": {
            "alive": fmt_up,
            "connection_probe_ms": fmt_ms,
            "port": formatter_port,
            "model": model,
            "loaded": loaded,
        },
        "dictionary": {"entries": dict_entries()},
        "history": {"mode": history_mode(&settings)},
        "uptime_s": started.elapsed().as_secs(),
        "settings_error": settings_error,
        "settings": settings,
        "runtime": runtime,
        "platform": crate::platform::diagnostics(),
    })
    .to_string()
}

fn load_settings() -> (Value, bool) {
    let path = crate::platform::settings_path();
    match std::fs::read_to_string(&path) {
        Ok(txt) => match serde_json::from_str::<Value>(&txt) {
            Ok(v) => {
                let invalid = serde_json::from_value::<crate::store::settings::Settings>(v.clone())
                    .is_err()
                    || v.get("schema_version").and_then(Value::as_u64).unwrap_or(2) > 2;
                (
                    serde_json::to_value(crate::store::settings::Settings::load())
                        .unwrap_or_else(|_| defaults_json()),
                    invalid,
                )
            }
            Err(_) => (
                serde_json::to_value(crate::store::settings::Settings::load())
                    .unwrap_or_else(|_| defaults_json()),
                true,
            ),
        },
        // Missing file = clean first-run defaults, not an error. Only a file
        // that exists but cannot be read/parsed is flagged to the UI.
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => (defaults_json(), false),
        Err(_) => (defaults_json(), true),
    }
}

fn defaults_json() -> Value {
    serde_json::to_value(crate::store::settings::Settings::default()).unwrap_or_else(|_| json!({}))
}

fn history_mode(settings: &Value) -> String {
    settings
        .get("history_mode")
        .and_then(Value::as_str)
        .unwrap_or("auto_delete_24h")
        .to_string()
}

/// TCP liveness + connect latency, hard-capped at 800 ms.
fn probe_tcp(port: u16) -> (bool, u128) {
    let addr: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)));
    let t0 = Instant::now();
    match TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
        Ok(_) => (true, t0.elapsed().as_millis()),
        Err(_) => (false, 0),
    }
}

/// Ollama keeps warm models listed at /api/ps. Pinned = our model present.
/// Any transport/parse problem counts as NOT pinned (never blocks the UI).
fn formatter_pinned(model: &str, port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/api/ps");
    let resp = match ureq::get(&url).timeout(Duration::from_millis(1200)).call() {
        Ok(r) => r,
        Err(_) => return false,
    };
    let mut txt = String::new();
    if resp
        .into_reader()
        .take(64 * 1024)
        .read_to_string(&mut txt)
        .is_err()
    {
        return false;
    }
    serde_json::from_str::<Value>(&txt)
        .ok()
        .and_then(|v| v.get("models").and_then(Value::as_array).cloned())
        .map(|models| {
            models.iter().any(|m| {
                m.get("name")
                    .and_then(Value::as_str)
                    .map(|n| n.starts_with(model))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Read-only COUNT(*) on the dictionary store; missing table/file = 0.
fn dict_entries() -> i64 {
    use rusqlite::OpenFlags;
    let path = crate::platform::data_dir().join("dictionary.sqlite");
    let conn = match rusqlite::Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    {
        Ok(c) => c,
        Err(_) => return 0,
    };
    conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_cannot_supply_headers_and_duplicate_headers_fail() {
        assert_eq!(
            header(
                "POST / HTTP/1.1\r\nHost: localhost:9393\r\n\r\nX-Parla-Control: 1",
                "x-parla-control"
            ),
            None
        );
        let mut input=std::io::Cursor::new(b"POST / HTTP/1.1\r\nHost: localhost:9393\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n");
        assert!(read_request(&mut input).unwrap_err().contains("duplicate"));
    }
    #[test]
    fn request_framing_is_exact_and_bounded() {
        for text in [
            "POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n{}",
        ] {
            assert!(read_request(&mut std::io::Cursor::new(text.as_bytes())).is_err());
        }
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEADER));
        assert!(read_request(&mut std::io::Cursor::new(huge.as_bytes())).is_err());
        let valid = "POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(
            read_request(&mut std::io::Cursor::new(valid.as_bytes())).unwrap(),
            valid
        );
    }
    #[test]
    fn settings_patch_preserves_unknown_values_and_enforces_schema() {
        let (v, s) =
            merge_settings(json!({"custom":"keep"}), json!({"hud_enabled":false})).unwrap();
        assert_eq!(v["custom"], "keep");
        assert!(!s.hud_enabled);
        assert!(
            merge_settings(json!({"schema_version":99}), json!({"hud_enabled":false})).is_err()
        );
        assert!(merge_settings(json!({}), json!({"hud_enabled":"no"})).is_err());
        assert!(merge_settings(json!({}), json!({"arbitrary":true})).is_err());
    }
}
