// Local whisper.cpp HTTP adapter: multipart PCM16 mono 16 kHz WAV.
// Configure the actual server/model paths; measure latency on the recipient machine.
use super::{AsrEngine, RawTranscript};
use std::io::Read;

pub struct WhisperServerClient {
    pub base_url: String,
    pub language: String,
}

impl WhisperServerClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            language: "en".into(),
        }
    }

    /// Build a WAV container (16 kHz mono i16) around raw PCM - the server's
    /// /inference endpoint expects a file upload.
    fn pcm_to_wav(pcm: &[i16]) -> Vec<u8> {
        const RATE: u32 = 16_000;
        let data_len = pcm.len() * 2;
        let mut wav = Vec::with_capacity(44 + data_len);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in pcm {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        wav
    }
}

impl AsrEngine for WhisperServerClient {
    fn transcribe(
        &self,
        pcm: &[i16],
        language: &str,
        hotwords: &[String],
    ) -> Result<RawTranscript, String> {
        let lang = if language.is_empty() {
            &self.language
        } else {
            language
        };
        let wav = super::wav::pcm16_mono_16k(pcm);

        // minimal multipart/form-data body (no external crate needed)
        let boundary = "parla-boundary-7d1a";
        let mut body = Vec::new();
        let mut part = |name: &str, filename: Option<&str>, content_type: &str, data: &[u8]| {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match filename {
                Some(f) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        };
        part("file", Some("audio.wav"), "audio/wav", &wav);
        part("response_format", None, "text/plain", b"text");
        part("language", None, "text/plain", lang.as_bytes());
        if !hotwords.is_empty() {
            // whisper.cpp 'prompt' param biases recognition (spec §7.6 place 1)
            let prompt = hotwords.join(", ");
            part("prompt", None, "text/plain", prompt.as_bytes());
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let url = format!("{}/inference", self.base_url);
        let req = ureq::post(&url)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .timeout(std::time::Duration::from_secs(
                (20 + pcm.len() as u64 / 8_000).clamp(30, 900),
            ));
        let resp = req
            .send_bytes(&body)
            .map_err(|e| format!("asr http: {e}"))?;
        let mut text = String::new();
        resp.into_reader()
            .take(128 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("asr read: {e}"))?;
        if text.len() > 128 * 1024 {
            return Err("ASR response exceeded the bounded 128 KiB limit".into());
        }
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return Err("empty transcript (silence?)".into()); // spec: discard near-silent clips
        }
        Ok(RawTranscript {
            text: trimmed,
            confidence: None, // server does not expose a validated score
            language: lang.to_string(),
        })
    }

    fn name(&self) -> &'static str {
        "whisper-server"
    }
}
