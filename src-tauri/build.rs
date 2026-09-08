fn main() {
    // Phase 1: tauri_build::build()
    println!("cargo:rerun-if-changed=tauri.conf.json");
}
