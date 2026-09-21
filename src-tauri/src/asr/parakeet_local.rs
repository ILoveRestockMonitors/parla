// Parakeet ASR backend client: HTTP to the local sherpa-onnx shim (:9293).
// Shim contract (assets/parakeet-shim.py): POST /inference raw WAV bytes ->
// 200 {"text"} | 5xx {"error"}; GET /health -> {"ok"} only once model loaded.
use super::RawTranscript;
use crate::store::settings::Settings;
use std::io::Read;

pub struct ParakeetClient {
    pub base_url: String,
    pub model_dir: String,
}

impl ParakeetClient {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            base_url: "http://127.0.0.1:9293".into(),
            model_dir: settings.parakeet_model_dir.clone(),
        }
    }

    /// All four weight files present? Cheap existence check gates activation.
    pub fn weights_present(&self) -> bool {
        let dir = std::path::Path::new(&self.model_dir);
        if !dir.is_dir() {
            return false;
        }
        let (mut enc, mut dec, mut join, mut tok) = (false, false, false, false);
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let low = e.file_name().to_string_lossy().to_lowercase();
                if low.ends_with(".onnx") {
                    if low.contains("encoder") {
                        enc = true;
                    } else if low.contains("joiner") {
                        join = true;
                    } else if low.contains("decoder") {
                        dec = true;
                    }
                } else if low == "tokens.txt" {
                    tok = true;
                }
            }
        }
        enc && dec && join && tok
    }

    pub fn alive(&self) -> bool {
        match ureq::get(&format!("{}/health", self.base_url))
            .timeout(std::time::Duration::from_millis(800))
            .call()
        {
            Ok(r) => {
                let mut text = String::new();
                r.into_reader()
                    .take(4097)
                    .read_to_string(&mut text)
                    .ok()
                    .filter(|_| text.len() <= 4096)
                    .map(|_| text)
                    .and_then(|s| {
                        let v: serde_json::Value = serde_json::from_str(&s).ok()?;
                        Some(health_matches(&v, &self.model_dir))
                    })
                    .unwrap_or(false)
            }
            Err(_) => false,
        }
    }

    pub fn runtime_directory(&self) -> Result<std::path::PathBuf, String> {
        // Never write generated code/logs beside installed, potentially
        // read-only model weights (especially /Applications and /opt).
        Ok(crate::platform::data_dir().join("runtime/parakeet"))
    }

    /// Bundled installations keep generated code/logs outside installed models.
    pub fn ensure_shim_deployed(&self) -> Result<std::path::PathBuf, String> {
        let root = self.runtime_directory()?;
        std::fs::create_dir_all(&root).map_err(|e| format!("mkdir {root:?}: {e}"))?;
        let dst = root.join("shim.py");
        let bytes = include_bytes!("../../assets/parakeet-shim.py");
        if std::fs::read(&dst).ok().as_deref() != Some(bytes) {
            crate::store::settings::atomic_write(&dst, bytes)
                .map_err(|e| format!("replace shim: {e}"))?;
        }
        Ok(dst)
    }
}

fn health_matches(value: &serde_json::Value, model_dir: &str) -> bool {
    let normalize = |path: &str| {
        #[cfg(windows)]
        {
            path.trim_start_matches(r"\\?\")
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        }
        #[cfg(not(windows))]
        {
            path.trim_end_matches('/').to_string()
        }
    };
    let expected = std::fs::canonicalize(model_dir)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| model_dir.into());
    value.get("ok").and_then(|v| v.as_bool()) == Some(true)
        && value.get("backend").and_then(|v| v.as_str()) == Some("parakeet")
        && value.get("protocol_version").and_then(|v| v.as_u64()) == Some(2)
        && value
            .get("model_directory")
            .and_then(|v| v.as_str())
            .is_some_and(|p| normalize(p) == normalize(&expected))
}

#[cfg(test)]
mod health_tests {
    use super::*;
    #[test]
    fn health_requires_ready_protocol_and_matching_model_directory() {
        let path = std::env::temp_dir().join("parla-missing-model-a");
        let expected = path.to_string_lossy();
        let mut value = serde_json::json!({"ok":true,"backend":"parakeet","protocol_version":2,"model_directory":expected});
        assert!(health_matches(&value, &expected));
        assert!(!health_matches(&value, "/different/model-b"));
        value["ok"] = serde_json::json!(false);
        assert!(!health_matches(&value, &expected));
        assert!(!health_matches(
            &serde_json::json!({"ok":true}),
            "C:\\model-a"
        ));
    }
}

impl super::AsrEngine for ParakeetClient {
    fn transcribe(
        &self,
        pcm: &[i16],
        _language: &str,
        _hotwords: &[String],
    ) -> Result<RawTranscript, String> {
        let wav = super::wav::pcm16_mono_16k(pcm);
        let resp = ureq::post(&format!("{}/inference", self.base_url))
            .timeout(std::time::Duration::from_secs(
                (20 + pcm.len() as u64 / 8_000).clamp(30, 900),
            ))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&wav)
            .map_err(|e| format!("parakeet http: {e}"))?;
        let mut text = String::new();
        resp.into_reader()
            .take(128 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("parakeet read: {e}"))?;
        if text.len() > 128 * 1024 {
            return Err("Parakeet response exceeded the limit".into());
        }
        let obj: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("parakeet json: {e}"))?;
        if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
            return Err(format!("parakeet: {err}"));
        }
        let out = obj
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if out.is_empty() {
            return Err("parakeet: empty transcript".into());
        }
        Ok(RawTranscript {
            text: out,
            confidence: None,
            language: "en".into(),
        })
    }

    fn name(&self) -> &'static str {
        "parakeet"
    }
}

impl ParakeetClient {
    /// 44-byte canonical PCM16 mono WAV header + payload (same as whisper_local).
    fn pcm_to_wav(pcm: &[i16]) -> Vec<u8> {
        let data_len = pcm.len() * 2;
        let mut b = Vec::with_capacity(44 + data_len);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&16_000u32.to_le_bytes());
        b.extend_from_slice(&32_000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in pcm {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }
}
