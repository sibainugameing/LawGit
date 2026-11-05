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
