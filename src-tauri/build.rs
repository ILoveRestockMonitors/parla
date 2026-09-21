fn main() {
    println!("cargo:rerun-if-env-changed=PARLA_BUILD_REVISION");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-changed=../VERSION");
    let revision = std::env::var("PARLA_BUILD_REVISION")
        .or_else(|_| std::env::var("GITHUB_SHA"))
        .unwrap_or_else(|_| "unrecorded-source-build".into());
    println!("cargo:rustc-env=PARLA_SOURCE_REVISION={revision}");
}
