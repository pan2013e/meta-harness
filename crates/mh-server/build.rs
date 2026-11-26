use std::{path::Path, process::Command};

fn main() -> std::io::Result<()> {
    track_build_commit();
    #[cfg(windows)]
    {
        let icon = "../../assets/icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .compile()?;
    }
    Ok(())
}

fn track_build_commit() {
    println!("cargo:rerun-if-env-changed=MH_BUILD_COMMIT");
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "packed-refs"].into_iter().chain(branch.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name])
            && Path::new(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    let commit = std::env::var("MH_BUILD_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]));
    if let Some(commit) = commit
        && commit.bytes().all(|b| b.is_ascii_hexdigit())
        && let Some(short) = commit.get(..6)
    {
        println!("cargo:rustc-env=MH_BUILD_COMMIT={short}");
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_owned())
}
