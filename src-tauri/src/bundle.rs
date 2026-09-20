//! Paths for the offline installer payload. Mutable user data stays in LocalAppData/Parla.
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct BundledPaths {
    pub whisper: String,
    pub whisper_model: String,
    pub parakeet_model: String,
    pub python: String,
}

impl BundledPaths {
    pub fn at(root: &Path) -> Option<Self> {
        let files = [
            "bundle-manifest.json",
            "engines/whisper/whisper-server.exe",
            "models/whisper/ggml-small.bin",
            "runtime/python/python.exe",
            "models/parakeet/encoder.int8.onnx",
            "models/parakeet/decoder.int8.onnx",
            "models/parakeet/joiner.int8.onnx",
            "models/parakeet/tokens.txt",
        ];
        if !files.iter().all(|p| {
            root.join(p)
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() > 0)
        }) {
            return None;
        }
        Some(Self {
            whisper: root.join(files[1]).to_string_lossy().into_owned(),
            whisper_model: root.join(files[2]).to_string_lossy().into_owned(),
            parakeet_model: root.join("models/parakeet").to_string_lossy().into_owned(),
            python: root.join(files[3]).to_string_lossy().into_owned(),
        })
    }
}

pub fn installed() -> Option<BundledPaths> {
    BundledPaths::at(std::env::current_exe().ok()?.parent()?)
}

pub fn initialize_settings() -> Result<(), String> {
    if installed().is_none() {
        return Err("Bundled speech files are incomplete; reinstall Parla.".into());
    }
    let path =
        PathBuf::from(std::env::var("LOCALAPPDATA").map_err(|_| "LOCALAPPDATA unavailable")?)
            .join("Parla/settings.json");
    write_new_settings(&path, &crate::store::settings::Settings::default())
}

fn write_new_settings(
    path: &Path,
    settings: &crate::store::settings::Settings,
) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().ok_or("Settings folder unavailable")?)
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_paths_require_all_assets_and_keep_spaces() {
        let root =
            std::env::temp_dir().join(format!("parla bundle paths test {}", std::process::id()));
        assert!(BundledPaths::at(&root).is_none());
        let files = [
            "bundle-manifest.json",
            "engines/whisper/whisper-server.exe",
            "models/whisper/ggml-small.bin",
            "runtime/python/python.exe",
            "models/parakeet/encoder.int8.onnx",
            "models/parakeet/decoder.int8.onnx",
            "models/parakeet/joiner.int8.onnx",
            "models/parakeet/tokens.txt",
        ];
        for name in files {
            let file = root.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"fixture").unwrap();
        }
        let paths = BundledPaths::at(&root).unwrap();
        assert_eq!(
            Path::new(&paths.python),
            root.join("runtime/python/python.exe")
        );
        std::fs::write(root.join(files[7]), b"").unwrap();
        assert!(BundledPaths::at(&root).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn initialization_never_overwrites_existing_personal_settings() {
        let root =
            std::env::temp_dir().join(format!("parla-bundle-settings-test-{}", std::process::id()));
        let path = root.join("settings.json");
        write_new_settings(&path, &crate::store::settings::Settings::default()).unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).is_ok()
        );
        std::fs::write(&path, b"preserve even an unreadable settings file").unwrap();
        write_new_settings(&path, &crate::store::settings::Settings::default()).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"preserve even an unreadable settings file"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
