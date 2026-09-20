//! Read-only dependency checks. No downloads, package installation or model loads.
use crate::store::settings::{AsrBackend, Settings};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GUIDE: &str =
    "https://github.com/ILoveRestockMonitors/parla/blob/main/docs/windows-download.md";
const PARAKEET_GUIDE: &str = "https://github.com/ILoveRestockMonitors/parla/blob/main/docs/windows-download.md#optional-parakeet-setup";
const WHISPER: &str = "https://github.com/ggml-org/whisper.cpp/releases";
const WHISPER_MODEL: &str = "https://huggingface.co/ggerganov/whisper.cpp";
const PYTHON: &str = "https://www.python.org/downloads/windows/";
const PARAKEET_MODEL: &str = "https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html";
const OLLAMA: &str = "https://ollama.com/download/windows";
const OLLAMA_MODELS: &str = "https://ollama.com/library";

fn item(
    id: &str,
    title: &str,
    state: &str,
    required: bool,
    detail: &str,
    links: &[(&str, &str)],
) -> Value {
    json!({"id":id,"title":title,"state":state,"required":required,"detail":detail,
        "links":links.iter().map(|(label,url)| json!({"label":label,"url":url})).collect::<Vec<_>>()})
}

fn nonempty_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn file_item(id: &str, title: &str, path: &str, detail: &str, links: &[(&str, &str)]) -> Value {
    let found = nonempty_file(Path::new(path));
    let mut row = item(
        id,
        title,
        if found { "found" } else { "not_found" },
        true,
        if found { "The configured file is available. Check Speech server readiness before dictating." } else { detail },
        links,
    );
    row["path"] = json!(path);
    row
}

fn parakeet_files_present(directory: &Path) -> bool {
    let mut found = [false; 4];
    if let Ok(entries) = directory.read_dir() {
        for entry in entries.flatten().filter(|e| nonempty_file(&e.path())) {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "tokens.txt" {
                found[3] = true;
            }
            for (index, part) in ["encoder", "decoder", "joiner"].iter().enumerate() {
                if name.ends_with(".onnx") && name.contains(part) {
                    found[index] = true;
                }
            }
        }
    }
    found.iter().all(|v| *v)
}

/// Resolve only locations this process can use; an undetected install may exist elsewhere.
fn executable_path(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        return nonempty_file(path).then(|| path.to_path_buf());
    }
    let filename = if program.to_ascii_lowercase().ends_with(".exe") {
        program.to_string()
    } else {
        format!("{program}.exe")
    };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        // Windows' Python store aliases are not an installed interpreter.
        .filter(|dir| {
            !dir.to_string_lossy()
                .to_ascii_lowercase()
                .contains("windowsapps")
        })
        .map(|dir| dir.join(&filename))
        .find(|path| nonempty_file(path))
}

#[derive(Clone, Copy)]
enum PythonCheck {
    Ready,
    PackagesMissing,
    Failed,
    NotFound,
}

fn check_python(program: &str) -> PythonCheck {
    let Some(path) = executable_path(program) else {
        return PythonCheck::NotFound;
    };
    let mut command = Command::new(path);
    command.args(["-B", "-c", "import sys\ntry:\n import numpy, sherpa_onnx\nexcept ModuleNotFoundError:\n sys.exit(10)\nexcept Exception:\n sys.exit(11)\n"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = command.spawn() else {
        return PythonCheck::Failed;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(0) => PythonCheck::Ready,
                    Some(10) => PythonCheck::PackagesMissing,
                    _ => PythonCheck::Failed,
                }
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return PythonCheck::Failed;
            }
        }
    }
}

fn model_names(value: &Value) -> Option<Vec<String>> {
    value
        .get("models")?
        .as_array()?
        .iter()
        .map(|entry| {
            entry
                .get("name")
                .or_else(|| entry.get("model"))?
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

fn same_model(left: &str, right: &str) -> bool {
    fn normalize(name: &str) -> String {
        if name.rsplit('/').next().unwrap_or(name).contains(':') {
            name.into()
        } else {
            format!("{name}:latest")
        }
    }
    normalize(left) == normalize(right)
}

fn ollama_models(port: u16) -> Option<Vec<String>> {
    let response = ureq::get(&format!("http://127.0.0.1:{port}/api/tags"))
        .timeout(Duration::from_millis(1200))
        .call()
        .ok()?;
    let mut text = String::new();
    response
        .into_reader()
        .take(256 * 1024 + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() > 256 * 1024 {
        return None;
    }
    model_names(&serde_json::from_str::<Value>(&text).ok()?)
}

fn ollama_items(
    model: &str,
    required: bool,
    installed: bool,
    models: Option<&[String]>,
) -> Vec<Value> {
    let state = if models.is_some() {
        "ready"
    } else if installed {
        "not_running"
    } else {
        "not_detected"
    };
    let detail = match state {
        "ready" => "Ollama is responding. It is used only for Polished cleanup.",
        "not_running" => "Ollama was found, but its local API is not responding. Open Ollama, then recheck.",
        _ => "Ollama is not responding and was not found in the usual locations. Open it if already installed, or download it for Polished cleanup.",
    };
    let model_state = match models {
        Some(names) if names.iter().any(|name| same_model(name, model)) => "found",
        Some(_) => "not_found",
        None => "unchecked",
    };
    vec![
        item(
            "ollama",
            "Ollama",
            state,
            required,
            detail,
            &[("Download Ollama", OLLAMA)],
        ),
        item(
            "ollama-model",
            &format!("Polishing model: {model}"),
            model_state,
            required,
            if models.is_none() {
                "Start Ollama and recheck to see whether this model is downloaded. Faithful mode works without it."
            } else if model_state == "found" {
                "Downloaded in Ollama. The model can load when Polished cleanup is used."
            } else {
                "Download the exact model named above in Ollama, then recheck. Faithful mode works without it."
            },
            &[
                ("Browse Ollama models", OLLAMA_MODELS),
                ("Setup instructions", GUIDE),
            ],
        ),
    ]
}

pub fn inspect(settings: &Settings) -> Value {
    let mut items = Vec::new();
    if settings.asr_backend_kind() == AsrBackend::Whisper {
        items.push(file_item("whisper", "Whisper speech engine", &settings.asr_server_exe,
            "Use a Windows x64 download containing whisper-server.exe and keep its matching DLLs together. If already installed elsewhere, update the configured path in settings.json.",
            &[("Download Whisper", WHISPER), ("Setup instructions", GUIDE)]));
        items.push(file_item("whisper-model", "Whisper speech model", &settings.asr_model_path,
            "Download ggml-small.bin, then configure its full file path in settings.json. A file being present does not prove the speech server has loaded it.",
            &[("Download a Whisper model", WHISPER_MODEL), ("Setup instructions", GUIDE)]));
    } else {
        let python = crate::asr::parakeet_python();
        let check = check_python(&python);
        let python_state = match check {
            PythonCheck::Ready | PythonCheck::PackagesMissing => "found",
            PythonCheck::NotFound => "not_detected",
            PythonCheck::Failed => "check_failed",
        };
        let mut python_item = item("python", "Python for Parakeet", python_state, true,
            if python_state == "found" { "Parla found its Python runtime. No separate Python installation is needed." }
            else { "Parakeet requires Python. Reinstall the bundled Parla app, or follow the manual Python 3.12 x64 setup guide and restart Parla." },
            &[("Download Python", PYTHON), ("Parakeet setup", PARAKEET_GUIDE)]);
        python_item["path"] = json!(python);
        items.push(python_item);
        items.push(item("sherpa", "sherpa-onnx and NumPy", match check {
                PythonCheck::Ready => "ready", PythonCheck::PackagesMissing => "not_found", _ => "unchecked" }, true,
            if matches!(check, PythonCheck::Ready) { "The packages needed for Parakeet are ready." }
            else { "These packages run Parakeet. Reinstall the bundled app, or follow the manual setup instructions for the Python environment Parla uses." },
            &[("Install Parakeet packages", PARAKEET_GUIDE)]));
        let model_found = parakeet_files_present(Path::new(&settings.parakeet_model_dir));
        let mut weights = item("parakeet-model", "Parakeet speech model", if model_found { "found" } else { "not_found" }, true,
            if model_found { "The speech model files are available. Check Speech server readiness before dictating." }
            else { "Reinstall the bundled app, or manually set up Parakeet TDT 0.6B v2 INT8 with matching encoder, decoder, joiner ONNX files and tokens.txt in the configured folder." },
            &[("Download Parakeet model", PARAKEET_MODEL), ("Parakeet setup", PARAKEET_GUIDE)]);
        weights["path"] = json!(settings.parakeet_model_dir);
        items.push(weights);
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let installed = nonempty_file(&Path::new(&local).join("Programs/Ollama/ollama.exe"))
        || executable_path("ollama").is_some();
    let models = ollama_models(settings.formatter_port);
    items.extend(ollama_items(
        &settings.formatter_model,
        settings.cleanup_mode == "polished",
        installed,
        models.as_deref(),
    ));
    json!({"engine":settings.asr_backend,"cleanup_mode":settings.cleanup_mode,"items":items,"guide_url":GUIDE})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_ollama_does_not_claim_the_model_is_missing() {
        let rows = ollama_items("test:3b", false, true, None);
        assert_eq!(rows[0]["state"], "not_running");
        assert_eq!(rows[1]["state"], "unchecked");
        assert_eq!(rows[0]["required"], false);
        assert_eq!(
            ollama_items("test:3b", true, false, None)[0]["state"],
            "not_detected"
        );
    }
    #[test]
    fn downloaded_models_need_not_be_loaded_and_tags_match_exactly() {
        let names = vec!["test:3b".into(), "other:latest".into()];
        assert_eq!(
            ollama_items("test:3b", true, false, Some(&names))[1]["state"],
            "found"
        );
        assert_eq!(
            ollama_items("test:3", true, false, Some(&names))[1]["state"],
            "not_found"
        );
        assert!(same_model("other", "other:latest"));
        assert!(!same_model("test:3b", "test:3b-extra"));
        assert!(!same_model("test:3b", "someone/test:3b"));
    }
    #[test]
    fn malformed_model_responses_are_unknown_not_empty() {
        assert_eq!(model_names(&json!({"models":[]})), Some(vec![]));
        assert!(model_names(&json!({"status":"ok"})).is_none());
        assert!(model_names(&json!({"models":[{}]})).is_none());
        assert_eq!(
            model_names(&json!({"models":[{"model":"test:3b"}]})),
            Some(vec!["test:3b".into()])
        );
    }
    #[test]
    fn missing_empty_or_directory_models_do_not_count_as_downloaded() {
        let dir = std::env::temp_dir().join(format!(
            "parla-setup-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("ggml-small.bin");
        assert!(!nonempty_file(&model));
        std::fs::write(&model, b"").unwrap();
        assert!(!nonempty_file(&model));
        std::fs::write(&model, b"model fixture").unwrap();
        assert!(nonempty_file(&model));
        assert!(!nonempty_file(&dir));
        for name in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"] {
            std::fs::write(dir.join(name), b"fixture").unwrap();
        }
        assert!(!parakeet_files_present(&dir));
        std::fs::create_dir(dir.join("tokens.txt")).unwrap();
        assert!(!parakeet_files_present(&dir));
        std::fs::remove_dir(dir.join("tokens.txt")).unwrap();
        std::fs::write(dir.join("tokens.txt"), b"tokens").unwrap();
        assert!(parakeet_files_present(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn ollama_probe_uses_downloaded_model_endpoint() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /api/tags "));
            let body = r#"{"models":[{"name":"test:3b"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        assert_eq!(ollama_models(port), Some(vec!["test:3b".into()]));
        thread.join().unwrap();
    }
}
