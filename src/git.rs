//! Thin wrapper around the `git` executable (LawGit never replaces Git).

use std::path::Path;
use std::process::Command;

use crate::check;
use crate::json::Json;
use crate::repo::Repo;

pub fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() { stdout.trim().to_string() } else { stderr };
        Err(format!("git {} failed: {}", args.first().copied().unwrap_or(""), detail))
    }
}

pub fn status(repo: &Repo) -> Result<String, String> {
    repo.require_initialized()?;
    run_git(&repo.root, &["status", "--porcelain=v1"])
}

/// Recent commits as JSON objects: hash, author, date, subject.
pub fn log(repo: &Repo, limit: usize) -> Result<Json, String> {
    repo.require_initialized()?;
    let n = format!("-n{}", limit.clamp(1, 500));
    let out = match run_git(
        &repo.root,
        &["log", n.as_str(), "--pretty=format:%H%x09%an%x09%aI%x09%s"],
    ) {
        Ok(o) => o,
        Err(e) => {
            if e.contains("does not have any commits") {
                String::new()
            } else {
                return Err(e);
            }
        }
    };
    let mut items = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() == 4 {
            items.push(Json::obj(vec![
                ("hash", Json::str(parts[0])),
                ("author", Json::str(parts[1])),
                ("date", Json::str(parts[2])),
                ("subject", Json::str(parts[3])),
            ]));
        }
    }
    Ok(Json::Arr(items))
}

/// Validate everything, stage `laws/` and `rules/`, and commit with `message`.
pub fn commit(repo: &Repo, message: &str) -> Result<String, String> {
    repo.require_initialized()?;
    if message.trim().is_empty() {
        return Err("invalid params: commit message must not be empty".to_string());
    }
    let report = check::check_all(repo)?;
    if !report.errors.is_empty() {
        return Err(format!(
            "invalid: validation failed, nothing committed: {}",
            report.errors.join("; ")
        ));
    }
    let mut add_args: Vec<&str> = vec!["add", "-A", "--"];
    if repo.laws_dir().exists() {
        add_args.push("laws");
    }
    if repo.rules_dir().exists() {
        add_args.push("rules");
    }
    run_git(&repo.root, &add_args)?;
    run_git(&repo.root, &["commit", "-m", message])
}
