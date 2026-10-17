//! Build identity (`meshway version`, the Server's netmap compiler input):
//! the release workflow passes `MESHWAY_GIT_DESCRIBE`; local builds ask Git
//! and fall back to `unknown`.
use std::process::Command;

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=MESHWAY_GIT_DESCRIBE");
    let describe = match std::env::var("MESHWAY_GIT_DESCRIBE") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_owned(),
        _ => {
            for path in ["HEAD", "logs/HEAD", "index"] {
                if let Some(file) = git(&["rev-parse", "--git-path", path]) {
                    println!("cargo:rerun-if-changed={file}");
                }
            }
            git(&["describe", "--tags", "--always", "--dirty", "--abbrev=8"])
                .unwrap_or_else(|| "unknown".into())
        }
    };
    println!("cargo:rustc-env=MESHWAY_BUILD_GIT={describe}");
    println!(
        "cargo:rustc-env=MESHWAY_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
}
