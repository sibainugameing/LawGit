//! Thin wrapper around the `git` executable (LawGit never replaces Git).

use std::path::Path;
use std::process::Command;

use crate::check;
use crate::json::Json;
use crate::repo::Repo;

const LOG_FORMAT: &str = "--pretty=format:%H%x09%an%x09%aI%x09%s";

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

/// True for a 4-40 digit hexadecimal commit id (never an option or a ref name).
pub fn is_hex_rev(rev: &str) -> bool {
    (4..=40).contains(&rev.len()) && rev.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_unborn(e: &str) -> bool {
    e.contains("does not have any commits") || e.contains("bad default revision")
}

/// Recent commits (optionally only those touching `paths`) as JSON objects:
/// hash, author, date, subject. A repository without commits yields an empty list.
pub fn log_paths(repo: &Repo, limit: usize, paths: &[&str]) -> Result<Json, String> {
    repo.require_initialized()?;
    let n = format!("-n{}", limit.clamp(1, 500));
    let mut args: Vec<&str> = vec!["log", n.as_str(), LOG_FORMAT];
    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().copied());
    }
    let out = match run_git(&repo.root, &args) {
        Ok(o) => o,
        Err(e) => {
            if is_unborn(&e) {
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

pub fn log(repo: &Repo, limit: usize) -> Result<Json, String> {
    log_paths(repo, limit, &[])
}

/// File content at a given commit (`rel` must be a repository-relative path built by the caller).
pub fn show_at(repo: &Repo, rev: &str, rel: &str) -> Result<String, String> {
    repo.require_initialized()?;
    if !is_hex_rev(rev) {
        return Err("invalid params: rev must be a 4-40 digit hexadecimal commit id".to_string());
    }
    let spec = format!("{}:{}", rev, rel);
    run_git(&repo.root, &["show", spec.as_str()])
        .map_err(|e| format!("not found: {} at revision {} ({})", rel, rev, e))
}

/// Commits (newest first) with the files each one changed. With `since`, only commits after it.
pub fn changes(repo: &Repo, since: Option<&str>, limit: usize) -> Result<Json, String> {
    repo.require_initialized()?;
    let n = format!("-n{}", limit.clamp(1, 500));
    let range = match since {
        Some(s) => {
            if !is_hex_rev(s) {
                return Err("invalid params: since must be a 4-40 digit hexadecimal commit id".to_string());
            }
            format!("{}..HEAD", s)
        }
        None => String::new(),
    };
    let mut args: Vec<&str> = vec![
        "log",
        n.as_str(),
        "--name-status",
        "--no-renames",
        "--pretty=format:%x01%H%x09%aI%x09%s",
    ];
    if since.is_some() {
        args.push(range.as_str());
    }
    let out = match run_git(&repo.root, &args) {
        Ok(o) => o,
        Err(e) => {
            if is_unborn(&e) {
                String::new()
            } else {
                return Err(e);
            }
        }
    };
    let mut items = Vec::new();
    for chunk in out.split('\u{1}') {
        if chunk.trim().is_empty() {
            continue;
        }
        let mut lines = chunk.lines();
        let head = lines.next().unwrap_or("");
        let parts: Vec<&str> = head.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let mut files = Vec::new();
        for l in lines {
            let l = l.trim_end();
            if l.is_empty() {
                continue;
            }
            let mut it = l.splitn(2, '\t');
            let st = it.next().unwrap_or("");
            let path = it.next().unwrap_or("");
            if !path.is_empty() {
                files.push(Json::obj(vec![("status", Json::str(st)), ("path", Json::str(path))]));
            }
        }
        items.push(Json::obj(vec![
            ("hash", Json::str(parts[0])),
            ("date", Json::str(parts[1])),
            ("subject", Json::str(parts[2])),
            ("files", Json::Arr(files)),
        ]));
    }
    let head = match run_git(&repo.root, &["rev-parse", "HEAD"]) {
        Ok(h) => Json::Str(h.trim().to_string()),
        Err(_) => Json::Null,
    };
    Ok(Json::obj(vec![("head", head), ("commits", Json::Arr(items))]))
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
