use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=src/process/ports.c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    // Use the installed SDK's libproc layout; copying socket unions into Rust
    // would silently couple ownership checks to guessed offsets.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("Unsupported macOS architecture {other}"),
    };
    assert!(
        Command::new("xcrun")
            .args([
                "--sdk",
                "macosx",
                "clang",
                "-arch",
                arch,
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
                "src/process/ports.c",
                "-o"
            ])
            .arg(out.join("ports.o"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libapphost_ports.a"))
            .arg(out.join("ports.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=apphost_ports");
}
