//! Records the git commit the app was built from as `APATURE_GIT_COMMIT`
//! (Help › Report a Bug… shows it). A value already in the environment wins;
//! outside a git checkout nothing is set and the app says "unknown".

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=APATURE_GIT_COMMIT");
    if std::env::var_os("APATURE_GIT_COMMIT").is_some() {
        return;
    }
    // Rebuild when HEAD moves: the worktree's HEAD, the branch it points at,
    // and packed refs. Only existing files, or cargo would rerun every time.
    let mut watch: Vec<PathBuf> = Vec::new();
    if let Some(d) = git(&["rev-parse", "--absolute-git-dir"]) {
        watch.push(Path::new(&d).join("HEAD"));
    }
    if let Some(common) = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
        watch.push(Path::new(&common).join("packed-refs"));
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            watch.push(Path::new(&common).join(r));
        }
    }
    for p in watch.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    if let Some(c) = git(&["rev-parse", "--short=7", "HEAD"]) {
        println!("cargo:rustc-env=APATURE_GIT_COMMIT={c}");
    }
}
