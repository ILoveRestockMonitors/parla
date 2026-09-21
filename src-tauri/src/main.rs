// Parla bootstrap. `--selftest` runs ASR->formatter end-to-end (no injection,
// no mic - safe while the user is at the keyboard). Default mode = live loop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod asr;
mod audio;
mod bundle;
mod cli;
mod command;
mod context;
mod dashboard;
mod dictionary;
mod formatter;
mod hotkey;
mod hud;
mod inject;
mod ipc;
mod pipeline;
mod platform;
mod runtime;
mod setup;
mod store;

fn main() {
    if let Err(error) = platform::initialize() {
        eprintln!("[parla] {error}");
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!(
            "Parla {} ({})",
            include_str!("../../VERSION").trim(),
            env!("PARLA_SOURCE_REVISION")
        );
        return;
    }
    if args.iter().any(|a| a == "--initialize-bundle") {
        if let Err(error) = bundle::initialize_settings() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--check-setup") {
        println!("{}", setup::inspect(&store::settings::Settings::load()));
        return;
    }
    #[cfg(windows)]
    if let Some(i) = args.iter().position(|a| a == "--preview-feedback") {
        let Some(directory) = args.get(i + 1) else {
            eprintln!("--preview-feedback requires an output directory");
            std::process::exit(1);
        };
        let directory = std::path::Path::new(directory);
        let result = std::fs::create_dir_all(directory)
            .and_then(|_| audio::beep::write_previews(directory))
            .and_then(|_| hud::write_previews(directory));
        if let Err(error) = result {
            eprintln!("[preview-feedback] {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--selftest") {
        run_selftest();
        return;
    }
    if args.iter().any(|a| a == "--export-prompt") {
        println!(
            "{}",
            serde_json::json!({"system": formatter::prompt::SYSTEM_PROMPT, "envelope_fields": ["mode","raw_transcript","context","language","vocabulary","options"]})
        );
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--format-json") {
        let code = run_format_json(args.get(i + 1).map(String::as_str).unwrap_or(""));
        std::process::exit(code);
    }
    if let Some(i) = args.iter().position(|a| a == "--replay") {
        let code = run_replay(
            args.get(i + 1).map(String::as_str).unwrap_or(""),
            args.iter()
                .position(|a| a == "--expected")
                .and_then(|j| args.get(j + 1).map(String::as_str)),
        );
        std::process::exit(code);
    }
    // Dictionary management verbs (parla add/list/remove) never start the
    // live loop; exit with the verb's status code.
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    run_live(args.iter().any(|a| a == "--dashboard"));
}

fn run_live(open_dashboard: bool) {
    let _instance = match platform::SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            if open_dashboard {
                dashboard::open_in_browser();
            }
            return;
        }
        Err(e) => {
            eprintln!("[parla] cannot establish single instance: {e}");
            return;
        }
    };
    let settings = store::settings::Settings::load();
    runtime::init(settings.clone());
    hud::spawn();
    hotkey::set_hotkeys(settings.hotkey_chords(), settings.toggle_hotkey_chord());
    match dashboard::spawn() {
        Err(e) => runtime::update(|s| s.error = Some(e)),
        Ok(()) => {
            let configured = platform::settings_path().is_file();
            if !configured || open_dashboard {
                dashboard::open_in_browser();
            }
        }
    }
    asr::set_backend(settings.asr_backend_kind());
    hotkey::spawn_listener();
    let mut mic = match audio::capture::MicCapture::open_with_settings(&settings) {
        Ok(mic) => mic,
        Err(e) => {
            runtime::update(|s| {
                s.mode = "error".into();
                s.error = Some(format!(
                    "Microphone unavailable: {e}. Check settings and restart Parla."
                ));
            });
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    };
    let folder = platform::data_dir();
    let _ = std::fs::create_dir_all(&folder);
    let dictionary_path = folder.join("dictionary.sqlite");
    let dict = match dictionary::Dictionary::open(&dictionary_path.to_string_lossy()) {
        Ok(dict) => dict,
        Err(e) => {
            runtime::update(|s| {
                s.error = Some(format!("Dictionary unavailable: {e}; using memory only."))
            });
            match dictionary::Dictionary::open(":memory:") {
                Ok(dict) => dict,
                Err(_) => return,
            }
        }
    };
    if let Err(e) = dict.seed_reliability_v2() {
        runtime::update(|s| s.error = Some(e));
    }
    // Warm the selected ASR on its supervised path. Recording remains usable
    // while it loads. Faithful mode does not load an unnecessary formatter.
    let warm_settings = settings.clone();
    std::thread::spawn(move || {
        if let Err(e) = asr::ensure_engine_for(&warm_settings, warm_settings.asr_backend_kind()) {
            runtime::update(|s| s.error = Some(e));
        }
    });
    if let Err(e) = pipeline::run_loop(&mut mic, &dict) {
        runtime::update(|s| s.error = Some(e));
    }
}
pub fn ensure_asr_server(settings: &store::settings::Settings) -> Result<(), String> {
    asr::ensure_engine_for(settings, store::settings::AsrBackend::Whisper).map(|_| ())
}

/// End-to-end ASR -> formatter proof. Reads a WAV (default: the SAPI-generated
/// test clip), transcribes via the whisper server, formats via Ollama, prints.
/// NEVER injects - foreground belongs to the user during tests.
fn run_selftest() {
    let wav_path = std::env::var("PARLA_TEST_WAV").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("parla_test.wav")
            .display()
            .to_string()
    });
    let code = run_replay(&wav_path, None);
    if code != 0 {
        std::process::exit(code);
    }
    println!("[selftest] completed shared replay path; injection skipped");
}

/// Format one JSON ContextEnvelope through the production bounded formatter.
/// This path never starts ASR, captures audio, or injects into a target.
fn run_format_json(path: &str) -> i32 {
    let raw = match read_bounded_file(path, 128 * 1024) {
        Ok(bytes) if bytes.len() <= 128 * 1024 => bytes,
        Ok(_) => {
            eprintln!("[format-json] input exceeds 128 KiB");
            return 1;
        }
        Err(e) => {
            eprintln!("[format-json] read: {e}");
            return 1;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[format-json] invalid JSON: {e}");
            return 1;
        }
    };
    let Some(obj) = value.as_object() else {
        eprintln!("[format-json] envelope must be an object");
        return 1;
    };
    let Some(raw_transcript) = obj.get("raw_transcript").and_then(|v| v.as_str()) else {
        eprintln!("[format-json] raw_transcript is required");
        return 1;
    };
    let ctx = obj.get("context").and_then(|v| v.as_object());
    let text = |key: &str| {
        ctx.and_then(|c| c.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let context = formatter::FieldContext {
        app: text("app"),
        app_category: text("app_category").unwrap_or_else(|| "other".into()),
        text_before: text("text_before"),
        selected_text: text("selected_text"),
        text_after: text("text_after"),
    };
    let options = obj.get("options").and_then(|v| v.as_object());
    let max_tokens = options
        .and_then(|o| o.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(256);
    let vocabulary = obj.get("vocabulary").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    });
    let envelope = formatter::ContextEnvelope {
        mode: obj
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("dictation")
            .into(),
        raw_transcript: raw_transcript.into(),
        context,
        user_style: obj.get("user_style").cloned(),
        language: obj
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en-US")
            .into(),
        vocabulary,
        options: formatter::Options {
            max_tokens,
            stream: false,
        },
    };
    let settings = store::settings::Settings::load();
    let mut client = formatter::local_llm::OllamaFormatter::new(
        settings.formatter_port,
        &settings.formatter_model,
    );
    client.num_ctx = settings.formatter_num_ctx;
    match formatter::format_complete(&client, &envelope) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::json!({"text": result.text, "metrics": {
                    "completion_reason": result.metadata.completion_reason,
                    "eval_count": result.metadata.eval_count,
                    "eval_duration_ns": result.metadata.eval_duration_ns,
                    "load_duration_ns": result.metadata.load_duration_ns,
                }})
            );
            0
        }
        Err(e) => {
            println!("{}", serde_json::json!({"error": e}));
            1
        }
    }
}

#[cfg(test)]
mod wav_tests {
    use super::read_wav_mono_16k;
    use std::io::Write;

    fn wav(bits: u16, rate: u32, channels: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36u32 + payload.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn path(name: &str, bytes: &[u8]) -> String {
        let p = std::env::temp_dir().join(format!("parla-wav-test-{name}.wav"));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p.display().to_string()
    }

    #[test]
    fn parser_accepts_exact_pcm_frame_count() {
        let p = path("valid", &wav(16, 16_000, 1, &[1, 0, 2, 0]));
        assert_eq!(read_wav_mono_16k(&p).unwrap(), vec![1, 2]);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn parser_rejects_truncated_container() {
        let mut bytes = wav(16, 16_000, 1, &[1, 0]);
        bytes.truncate(bytes.len() - 1);
        let p = path("truncated", &bytes);
        assert!(read_wav_mono_16k(&p).is_err());
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn parser_rejects_wrong_format() {
        let p = path("wrong-format", &wav(8, 8_000, 2, &[1, 2]));
        assert!(read_wav_mono_16k(&p).is_err());
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn parser_rejects_odd_data_bytes() {
        let p = path("odd-data", &wav(16, 16_000, 1, &[1]));
        assert!(read_wav_mono_16k(&p).is_err());
        let _ = std::fs::remove_file(p);
    }
}

/// Minimal RIFF/WAVE parser: PCM 16-bit mono 16 kHz only (what we produce).
fn read_wav_mono_16k(path: &str) -> Result<Vec<i16>, String> {
    let raw = read_bounded_file(path, 40 * 1024 * 1024).map_err(|e| format!("open: {e}"))?;
    if raw.len() < 12 || &raw[0..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    if u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize + 8 != raw.len() {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12usize;
    let (mut fmt_pcm, mut fmt_mono, mut fmt_rate, mut fmt_bits) = (false, false, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= raw.len() {
        let id = &raw[pos..pos + 4];
        let size = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = raw.get(pos + 8..pos + 8 + size).ok_or("truncated chunk")?;
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err("truncated fmt chunk".into());
                }
                fmt_pcm = u16::from_le_bytes(body[0..2].try_into().unwrap()) == 1;
                fmt_mono = u16::from_le_bytes(body[2..4].try_into().unwrap()) == 1;
                fmt_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                fmt_bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + size + (size & 1); // chunks are word-aligned
    }
    if pos != raw.len() {
        return Err("trailing incomplete WAV chunk".into());
    }
    if !fmt_pcm || !fmt_mono || fmt_rate != 16_000 || fmt_bits != 16 {
        return Err(format!(
            "expected PCM mono 16k 16-bit (got pcm={fmt_pcm} mono={fmt_mono} rate={fmt_rate} bits={fmt_bits})"
        ));
    }
    let bytes = data.ok_or("no data chunk")?;
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return Err("data chunk is empty or has odd byte count".into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|p| i16::from_le_bytes(p.try_into().unwrap()))
        .collect())
}

fn read_bounded_file(path: &str, limit: usize) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn run_replay(path: &str, expected_path: Option<&str>) -> i32 {
    let pcm = match read_wav_mono_16k(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[replay] {e}");
            return 2;
        }
    };
    let settings = store::settings::Settings::load();
    let dict = match dictionary::Dictionary::open(&cli::dict_store_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[replay] {e}");
            return 3;
        }
    };
    let now = std::time::Instant::now();
    let clip = audio::capture::AudioClip {
        samples: pcm.iter().map(|v| *v as f32 / 32768.0).collect(),
        sample_rate: 16_000,
        started: now,
        ended: now,
    };
    let result = pipeline::replay_clip(clip, settings, dict.all_entries());
    if let Some(e) = result.error {
        eprintln!("[replay] {e}");
        return 5;
    }
    let output = result
        .final_text
        .or(result.normalized)
        .or(result.raw)
        .unwrap_or_default();
    println!("[replay] final: {}", output);
    if let Some(p) = expected_path {
        let exp = match std::fs::read_to_string(p) {
            Ok(x) => x.trim().to_string(),
            Err(e) => {
                eprintln!("[replay] expected: {e}");
                return 6;
            }
        };
        if exp != output {
            eprintln!("[replay] mismatch");
            return 7;
        }
        println!("[replay] matched expected");
    }
    0
}
