fn main() {
    // ggml-metal (whisper.cpp) uses `@available`, which needs `___isPlatformVersionAtLeast` from clang's runtime. Rust
    // links with -nodefaultlibs and its compiler_builtins is built for a newer macOS than our deployment target (14),
    // so link the toolchain's libclang_rt.osx explicitly.
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("clang")
        .arg("--print-file-name=libclang_rt.osx.a")
        .output()
    {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if std::path::Path::new(&path).is_absolute() {
            println!("cargo:rustc-link-arg-bins={path}");
        }
    }
    tauri_build::build()
}
