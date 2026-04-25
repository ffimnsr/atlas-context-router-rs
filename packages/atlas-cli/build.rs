use std::process::Command;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

fn main() {
    let hash = git_output(["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let commit_date =
        git_output(["show", "-s", "--format=%cI", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let dirty = git_is_dirty();
    let cargo_profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());
    let rustc_version = rustc_version();

    let build_date = build_date();
    let long_version = format!(
        "{pkg_version} (git {hash}{dirty_suffix}, commit {commit_date}, built {build_date}, profile {cargo_profile}, {rustc_version})",
        dirty_suffix = if dirty { " dirty" } else { "" },
        pkg_version = env!("CARGO_PKG_VERSION")
    );

    println!("cargo:rustc-env=GIT_HASH={hash}");
    println!("cargo:rustc-env=GIT_COMMIT_DATE={commit_date}");
    println!(
        "cargo:rustc-env=GIT_DIRTY={}",
        if dirty { "true" } else { "false" }
    );
    println!("cargo:rustc-env=CARGO_PROFILE={cargo_profile}");
    println!("cargo:rustc-env=RUSTC_VERSION={rustc_version}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");
    println!("cargo:rustc-env=ATLAS_LONG_VERSION={long_version}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

fn build_date() -> String {
    let build_time = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        .unwrap_or_else(OffsetDateTime::now_utc);

    build_time
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn git_output<const N: usize>(args: [&str; N]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|value| value.trim().to_owned())
            } else {
                None
            }
        })
}

fn git_is_dirty() -> bool {
    git_output(["status", "--short"])
        .map(|output| !output.is_empty())
        .unwrap_or(false)
}

fn rustc_version() -> String {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|value| value.trim().to_owned())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "rustc unknown".to_owned())
}
