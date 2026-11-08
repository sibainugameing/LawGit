//! Automatic commits: every change made through LawGit (CLI or API) can be
//! recorded as exactly one Git commit that contains only the touched files.
//!
//! The setting is resolved in this order:
//!   1. `--commit` / `--no-commit` (stored in `Repo::auto_commit`)
//!   2. environment variable `LAWGIT_AUTOCOMMIT` (1/true/on/yes or 0/false/off/no)
//!   3. `git config lawgit.autocommit` of the repository (`lawgit config autocommit on`)
//!   4. off

use std::env;

use crate::git;
use crate::json::Json;
use crate::repo::Repo;

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

pub fn enabled(repo: &Repo) -> bool {
    if let Some(v) = repo.auto_commit {
        return v;
    }
    if let Ok(v) = env::var("LAWGIT_AUTOCOMMIT") {
        if let Some(b) = parse_bool(&v) {
            return b;
        }
    }
    match git::run_git(&repo.root, &["config", "--bool", "--get", "lawgit.autocommit"]) {
        Ok(o) => o.trim() == "true",
        Err(_) => false,
    }
}

/// Persist the setting in the repository's local Git config (not committed, not shared).
pub fn set_config(repo: &Repo, on: bool) -> Result<(), String> {
    if !repo.root.join(".git").exists() {
        return Err(format!("{} is not a Git repository", repo.root.display()));
    }
    let value = if on { "true" } else { "false" };
    git::run_git(&repo.root, &["config", "--local", "lawgit.autocommit", value])?;
    Ok(())
}

pub fn default_message(action: &str, name: &str, uuid_s: &str) -> String {
    format!("{}: {} [{}]", action, name, uuid_s)
}

pub fn clean_message(m: &str) -> Result<String, String> {
    let t: String = m
        .chars()
        .map(|c| if c == '\n' || !c.is_control() { c } else { ' ' })
        .collect();
    let t = t.trim().to_string();
    if t.is_empty() {
        return Err("invalid params: commit message must not be empty".to_string());
    }
    Ok(t.chars().take(2000).collect())
}

fn with_hint(e: String) -> String {
    if e.contains("Please tell me who you are")
        || e.contains("empty ident")
        || e.contains("unable to auto-detect email")
    {
        format!("{} (configure git user.name and user.email)", e)
    } else {
        e
    }
}

/// Commit exactly `rel_paths` (repository-relative, under `laws/` or `rules/`; deletions included)
/// if auto-commit is enabled. Other staged or modified files are left untouched.
///
/// * `Ok(None)`: disabled, or nothing changed in those paths
/// * `Ok(Some(hash))`: committed (short hash)
/// * `Err(msg)`: the file change itself is already saved, but the commit failed
pub fn commit_change(repo: &Repo, rel_paths: &[String], message: &str) -> Result<Option<String>, String> {
    if !enabled(repo) {
        return Ok(None);
    }
    let msg = clean_message(message)?;
    if !repo.root.join(".git").exists() {
        return Err(format!(
            "auto-commit is enabled but {} is not a Git repository",
            repo.root.display()
        ));
    }
    for p in rel_paths {
        let ok = (p.starts_with("laws/") || p.starts_with("rules/")) && !p.contains("..") && !p.contains('\\');
        if !ok {
            return Err(format!("internal: refusing to commit unexpected path '{}'", p));
        }
    }
    let mut add: Vec<&str> = vec!["add", "-A", "--"];
    for p in rel_paths {
        add.push(p.as_str());
    }
    git::run_git(&repo.root, &add).map_err(with_hint)?;

    let mut status: Vec<&str> = vec!["status", "--porcelain", "--"];
    for p in rel_paths {
        status.push(p.as_str());
    }
    let st = git::run_git(&repo.root, &status).map_err(with_hint)?;
    if st.trim().is_empty() {
        return Ok(None);
    }

    let mut commit: Vec<&str> = vec!["commit", "-q", "-m", msg.as_str(), "--"];
    for p in rel_paths {
        commit.push(p.as_str());
    }
    git::run_git(&repo.root, &commit).map_err(with_hint)?;
    let hash = git::run_git(&repo.root, &["rev-parse", "--short", "HEAD"]).map_err(with_hint)?;
    Ok(Some(hash.trim().to_string()))
}

/// JSON description of a `commit_change` outcome for API responses.
pub fn commit_json(r: &Result<Option<String>, String>, message: &str) -> Json {
    match r {
        Ok(Some(h)) => Json::obj(vec![
            ("hash", Json::str(h)),
            ("message", Json::str(message.lines().next().unwrap_or(""))),
        ]),
        Ok(None) => Json::Null,
        Err(e) => Json::obj(vec![("error", Json::str(e))]),
    }
}

/// Append `"commit": ...` to a JSON object result.
pub fn attach(result: Json, commit: Json) -> Json {
    match result {
        Json::Obj(mut pairs) => {
            pairs.push(("commit".to_string(), commit));
            Json::Obj(pairs)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("ON"), Some(true));
        assert_eq!(parse_bool(" 0 "), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn message_cleaning() {
        assert!(clean_message("  \n ").is_err());
        assert_eq!(clean_message("a\u{1b}b\nc").unwrap(), "a b\nc");
        assert_eq!(default_message("法令を追加", "道路交通法", "u"), "法令を追加: 道路交通法 [u]");
    }

    #[test]
    fn attach_adds_commit_field() {
        let j = attach(Json::obj(vec![("a", Json::Null)]), Json::str("x"));
        assert_eq!(j.get("commit").and_then(|v| v.as_str()), Some("x"));
    }
}
