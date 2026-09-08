// Engine selection + warm-up orchestration.
// v1: single local engine (whisper.cpp server on CUDA). Cloud engines Phase 8.
use super::WhisperServerClient;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AsrSelection {
    /// ggml-small via local whisper-server (current default)
    WhisperSmallServer,
    // Phase 8: DeepgramNova3 (streaming during hold)
}

impl AsrSelection {
    pub fn engine(self, server_port: u16) -> WhisperServerClient {
        match self {
            AsrSelection::WhisperSmallServer => WhisperServerClient::new(server_port),
        }
    }
}
