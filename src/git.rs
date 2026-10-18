//! Commit and push the generated files using the system `git` binary, so the
//! user's git config and SSH agent are used as-is.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::{Command, Output};

const COMMIT_MESSAGE: &str = "Update listening history";

pub fn is_repo() -> bool {
    git(&["rev-parse", "--is-inside-work-tree"]).is_ok_and(|o| o.status.success())
}

/// Stage `files`, and if anything changed commit them and push.
pub fn commit_and_push(files: &[&Path]) -> Result<()> {
    let paths: Vec<&str> = files.iter().filter_map(|p| p.to_str()).collect();
    run(&[&["add", "--"], paths.as_slice()].concat())?;

    let diff = git(&[&["diff", "--cached", "--quiet", "--"], paths.as_slice()].concat())?;
    if diff.status.success() {
        tracing::info!("git: nothing to commit");
        return Ok(());
    }
    run(&[&["commit", "-m", COMMIT_MESSAGE, "--"], paths.as_slice()].concat())?;
    run(&["push"])?;
    tracing::info!("git: committed and pushed");
    Ok(())
}

fn git(args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .output()
        .context("failed to run git")
}

fn run(args: &[&str]) -> Result<()> {
    let out = git(args)?;
    if !out.status.success() {
        bail!(
            "git {} failed ({}): {}",
            args[0],
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
