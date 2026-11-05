use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{read_law, validate_iso_date, validate_repository};

/// The source supplied by an external editor or application.
#[derive(Debug, Clone)]
pub enum AmendmentChange {
    /// Replace the complete XML document for the existing law.
    CompleteXml(String),
    /// Apply a unified diff whose target is laws/<UUID>.xml.
    UnifiedDiff(String),
}

/// Data required to apply one amendment and create its Git commit.
#[derive(Debug, Clone)]
pub struct Amendment {
    pub uuid: String,
    pub change: AmendmentChange,
    /// Promulgation date of the amending law. Git author and committer dates use this date.
    pub promulgated_date: String,
    /// One or more commencement dates. Multiple dates support staggered commencement.
    pub effective_dates: Vec<String>,
    pub message: String,
}

/// Apply a complete XML document or unified diff and commit only the affected law file.
///
/// The amendment's promulgation date is recorded as 00:00 JST in both Git timestamps.
/// Effective dates are stored as commit-message trailers.
pub fn amend_law(repo: &Path, amendment: &Amendment) -> Result<String> {
    if !repo.join(".git").exists() {
        bail!("{} is not a Git repository; run lawgit init first", repo.display());
    }
    if amendment.message.trim().is_empty() {
        bail!("amendment commit message must not be empty");
    }
    validate_iso_date(&amendment.promulgated_date, "promulgated-date")?;
    for date in &amendment.effective_dates {
        validate_iso_date(date, "effective-date")?;
    }

    let (law, original_xml) = read_law(repo, &amendment.uuid)
        .with_context(|| format!("cannot load target law {}", amendment.uuid))?;
    let relative_path = PathBuf::from("laws").join(format!("{}.xml", law.uuid));
    let absolute_path = repo.join(&relative_path);
    ensure_clean_tracked_file(repo, &relative_path)?;

    match &amendment.change {
        AmendmentChange::CompleteXml(xml) => {
            fs::write(&absolute_path, xml)
                .with_context(|| format!("cannot write {}", absolute_path.display()))?;
        }
        AmendmentChange::UnifiedDiff(patch) => {
            run_git_apply(repo, &relative_path, patch, true)?;
            run_git_apply(repo, &relative_path, patch, false)?;
        }
    }

    let validation = (|| -> Result<()> {
        let updated = validate_repository(repo)?;
        if !updated.iter().any(|item| item.uuid == law.uuid) {
            bail!("amendment removed the target law from the repository");
        }
        let updated_xml = fs::read_to_string(&absolute_path)
            .with_context(|| format!("cannot read {}", absolute_path.display()))?;
        if updated_xml == original_xml {
            bail!("amendment did not change the target XML document");
        }
        Ok(())
    })();

    if let Err(error) = validation {
        fs::write(&absolute_path, &original_xml)
            .with_context(|| format!("failed to restore {}", absolute_path.display()))?;
        return Err(error);
    }

    let add_output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("add")
        .arg("--")
        .arg(&relative_path)
        .output()
        .context("Git is required but could not be executed")?;
    if !add_output.status.success() {
        bail!(
            "git add failed: {}",
            String::from_utf8_lossy(&add_output.stderr).trim()
        );
    }

    let git_date = format!("{}T00:00:00+09:00", amendment.promulgated_date);
    let mut trailers = vec![format!(
        "LawGit-Amendment-Promulgated-Date: {}",
        amendment.promulgated_date
    )];
    for date in &amendment.effective_dates {
        trailers.push(format!("LawGit-Effective-Date: {date}"));
    }

    let commit_output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("commit")
        .arg("--only")
        .arg("--date")
        .arg(&git_date)
        .arg("-m")
        .arg(amendment.message.trim())
        .arg("-m")
        .arg(trailers.join("\n"))
        .arg("--")
        .arg(&relative_path)
        .env("GIT_AUTHOR_DATE", &git_date)
        .env("GIT_COMMITTER_DATE", &git_date)
        .output()
        .context("Git is required but could not be executed")?;

    if !commit_output.status.success() {
        bail!(
            "git commit failed (the validated XML remains in the working tree): {}",
            String::from_utf8_lossy(&commit_output.stderr).trim()
        );
    }

    let hash_output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .context("failed to read the created Git commit")?;
    if !hash_output.status.success() {
        bail!(
            "amendment was committed, but its hash could not be read: {}",
            String::from_utf8_lossy(&hash_output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&hash_output.stdout).trim().to_owned())
}

fn ensure_clean_tracked_file(repo: &Path, relative_path: &Path) -> Result<()> {
    let tracked = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("ls-files")
        .arg("--error-unmatch")
        .arg("--")
        .arg(relative_path)
        .output()
        .context("Git is required but could not be executed")?;
    if !tracked.status.success() {
        bail!("the target law file must be committed to Git before it can be amended");
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=all")
        .arg("--")
        .arg(relative_path)
        .output()
        .context("failed to inspect the target law file's Git status")?;
    if !status.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        );
    }
    if !String::from_utf8_lossy(&status.stdout).trim().is_empty() {
        bail!("the target law file has uncommitted changes; commit or stash them before applying an amendment");
    }
    Ok(())
}

fn run_git_apply(repo: &Path, relative_path: &Path, patch: &str, check: bool) -> Result<()> {
    let include = relative_path
        .to_str()
        .context("law file path is not valid UTF-8")?;
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).arg("apply");
    if check {
        command.arg("--check");
    }
    command.arg(format!("--include={include}")).arg("-");
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command.spawn().context("Git is required but could not be executed")?;
    {
        let mut stdin = child.stdin.take().context("failed to open git apply stdin")?;
        stdin.write_all(patch.as_bytes()).context("failed to send unified diff to git apply")?;
    }
    let output = child.wait_with_output().context("failed while applying unified diff")?;
    if !output.status.success() {
        bail!(
            "git apply{} failed: {}",
            if check { " --check" } else { "" },
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_law, init_repository, NewLaw};
    use std::process::Command;
    use uuid::Uuid;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn complete_xml_amendment_creates_a_dated_commit() {
        let repo = std::env::temp_dir().join(format!("lawgit-amend-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo).unwrap();
        init_repository(&repo).unwrap();
        git(&repo, &["config", "user.name", "LawGit Test"]);
        git(&repo, &["config", "user.email", "lawgit-test@example.invalid"]);

        let law = create_law(
            &repo,
            &NewLaw {
                name: "Test Act".to_owned(),
                law_type: "法律".to_owned(),
                law_number: None,
                e_gov_id: None,
                promulgated_date: Some("2026-01-15".to_owned()),
                effective_date: Some("2026-04-01".to_owned()),
            },
        )
        .unwrap();

        let xml = fs::read_to_string(&law.path).unwrap();
        let initial_xml = xml.replace(
            "<body />",
            "<body><article><text>Before</text></article></body>",
        );
        fs::write(&law.path, &initial_xml).unwrap();

        let pathspec = format!("laws/{}.xml", law.uuid);
        git(&repo, &["add", "--", pathspec.as_str()]);
        git(&repo, &["commit", "-m", "Initial law version"]);

        let amended_xml = initial_xml.replace("Before", "After");
        let commit = amend_law(
            &repo,
            &Amendment {
                uuid: law.uuid.clone(),
                change: AmendmentChange::CompleteXml(amended_xml),
                promulgated_date: "2026-11-05".to_owned(),
                effective_dates: vec!["2026-12-01".to_owned(), "2027-04-01".to_owned()],
                message: "Test amendment".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(commit, git(&repo, &["rev-parse", "HEAD"]));
        let author_date = git(&repo, &["log", "-1", "--format=%aI"]);
        let committer_date = git(&repo, &["log", "-1", "--format=%cI"]);
        assert!(author_date.starts_with("2026-11-05T00:00:00"));
        assert!(committer_date.starts_with("2026-11-05T00:00:00"));

        let message = git(&repo, &["log", "-1", "--format=%B"]);
        assert!(message.contains("LawGit-Amendment-Promulgated-Date: 2026-11-05"));
        assert!(message.contains("LawGit-Effective-Date: 2026-12-01"));
        assert!(message.contains("LawGit-Effective-Date: 2027-04-01"));

        let changed_files = git(&repo, &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]);
        assert_eq!(changed_files, format!("laws/{}.xml", law.uuid));

        fs::remove_dir_all(repo).unwrap();
    }
}


#[cfg(test)]
mod unified_diff_tests {
    use super::*;
    use crate::{create_law, init_repository, NewLaw};
    use std::process::Command;
    use uuid::Uuid;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn unified_diff_amendment_is_applied_to_the_requested_law() {
        let repo = std::env::temp_dir().join(format!("lawgit-patch-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo).unwrap();
        init_repository(&repo).unwrap();
        git(&repo, &["config", "user.name", "LawGit Test"]);
        git(&repo, &["config", "user.email", "lawgit-test@example.invalid"]);

        let law = create_law(
            &repo,
            &NewLaw {
                name: "Patch Test Act".to_owned(),
                law_type: "法律".to_owned(),
                law_number: None,
                e_gov_id: None,
                promulgated_date: None,
                effective_date: None,
            },
        )
        .unwrap();
        let xml = fs::read_to_string(&law.path).unwrap();
        let initial_xml = xml.replace(
            "<body />",
            "<body><article><text>Before</text></article></body>",
        );
        fs::write(&law.path, &initial_xml).unwrap();
        let relative_path = format!("laws/{}.xml", law.uuid);
        git(&repo, &["add", "--", relative_path.as_str()]);
        git(&repo, &["commit", "-m", "Initial law version"]);

        let amended_xml = initial_xml.replace("Before", "After");
        fs::write(&law.path, &amended_xml).unwrap();
        let diff_output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["diff", "--"])
            .arg(&relative_path)
            .output()
            .unwrap();
        assert!(diff_output.status.success());
        let patch = String::from_utf8(diff_output.stdout).unwrap();
        assert!(patch.contains(&relative_path));

        fs::write(&law.path, &initial_xml).unwrap();
        amend_law(
            &repo,
            &Amendment {
                uuid: law.uuid.clone(),
                change: AmendmentChange::UnifiedDiff(patch),
                promulgated_date: "2026-11-06".to_owned(),
                effective_dates: vec!["2026-12-15".to_owned()],
                message: "Patch-based amendment".to_owned(),
            },
        )
        .unwrap();

        let show_spec = format!("HEAD:{relative_path}");
        let committed_xml = git(&repo, &["show", show_spec.as_str()]);
        assert!(committed_xml.contains("After"));
        assert!(!committed_xml.contains("Before"));

        fs::remove_dir_all(repo).unwrap();
    }
}
