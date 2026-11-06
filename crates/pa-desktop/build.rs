//! Link the native updater only in macOS builds that package the Sparkle SDK.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=SPARKLE_SDK");
    println!("cargo:rerun-if-changed=macos/Updater.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Some(sdk) = env::var_os("SPARKLE_SDK").map(PathBuf::from) else {
        return;
    };
    let object = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR")).join("Updater.o");
    let status = Command::new("xcrun")
        .args(["--sdk", "macosx", "clang", "-target"])
        .arg(env::var("TARGET").expect("Cargo target"))
        .args([
            "-mmacosx-version-min=13.0",
            "-fobjc-arc",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-F",
        ])
        .arg(&sdk)
        .args(["-c", "macos/Updater.m", "-o"])
        .arg(&object)
        .status()
        .expect("run the macOS compiler");
    assert!(status.success(), "failed to compile the Sparkle updater");
    // Keep library consumers and Rune's headless tests independent of app bundle layout.
    for arg in [
        object.display().to_string(),
        format!("-F{}", sdk.display()),
        "-Wl,-framework,Sparkle".into(),
        "-Wl,-rpath,@executable_path/../Frameworks".into(),
    ] {
        println!("cargo:rustc-link-arg-bin=prime-agent-ui={arg}");
    }
}
