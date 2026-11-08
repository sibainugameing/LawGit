//! Repository layout: `laws/<UUID>.xml` and `rules/<UUID>.xml`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::write_new;
use crate::uuid;

pub struct Repo {
    pub root: PathBuf,
    /// Per-process override of the auto-commit setting (`--commit` / `--no-commit`).
    pub auto_commit: Option<bool>,
}

/// Valid items plus human-readable problems found while scanning a directory.
pub struct Report<T> {
    pub items: Vec<T>,
    pub errors: Vec<String>,
}

impl Repo {
    pub fn new<P: Into<PathBuf>>(root: P) -> Repo {
        Repo {
            root: root.into(),
            auto_commit: None,
        }
    }

    pub fn with_auto_commit(mut self, value: Option<bool>) -> Repo {
        self.auto_commit = value;
        self
    }

    pub fn laws_dir(&self) -> PathBuf {
        self.root.join("laws")
    }

    pub fn rules_dir(&self) -> PathBuf {
        self.root.join("rules")
    }

    pub fn require_initialized(&self) -> Result<(), String> {
        if self.laws_dir().is_dir() {
            Ok(())
        } else {
            Err(format!(
                "{} is not a LawGit repository (laws/ not found); run `lawgit --repo <dir> init` first",
                self.root.display()
            ))
        }
    }

    /// A UUID not used by any existing law or rule file.
    pub fn fresh_uuid(&self) -> Result<String, String> {
        for _ in 0..16 {
            let u = uuid::new_v4();
            let name = format!("{}.xml", u);
            if !self.laws_dir().join(&name).exists() && !self.rules_dir().join(&name).exists() {
                return Ok(u);
            }
        }
        Err("internal: could not generate an unused UUID".to_string())
    }

    /// Create directories, helper files and run `git init`. Returns messages to show.
    pub fn init(&self) -> Result<Vec<String>, String> {
        let mut msgs = Vec::new();
        fs::create_dir_all(&self.root)
            .map_err(|e| format!("cannot create {}: {}", self.root.display(), e))?;
        for dir in [self.laws_dir(), self.rules_dir()] {
            fs::create_dir_all(&dir)
                .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
            let keep = dir.join(".gitkeep");
            if !keep.exists() {
                write_new(&keep, "")?;
            }
        }
        let readme = self.root.join("README.md");
        if !readme.exists() {
            write_new(
                &readme,
                "# Legal data repository\n\nManaged with LawGit.\n\n- `laws/<UUID>.xml` legal documents\n- `rules/<UUID>.xml` user-authored rules\n",
            )?;
        }
        let attrs = self.root.join(".gitattributes");
        if !attrs.exists() {
            write_new(&attrs, "*.xml text eol=lf\n")?;
        }
        if self.root.join(".git").exists() {
            msgs.push(format!("Git repository already exists in {}", self.root.display()));
        } else {
            let out = Command::new("git").arg("init").arg(&self.root).output();
            match out {
                Ok(o) if o.status.success() => {
                    msgs.push(format!("Initialized Git repository in {}", self.root.display()))
                }
                Ok(o) => msgs.push(format!(
                    "warning: git init failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => msgs.push(format!("warning: cannot run git ({}); run `git init` manually", e)),
            }
        }
        msgs.push(format!("Created {} and {}", self.laws_dir().display(), self.rules_dir().display()));
        Ok(msgs)
    }
}

/// All `*.xml` files in `dir` (sorted) plus problems for unexpected entries.
/// A missing directory is treated as empty. Dot-files are ignored.
pub fn xml_files(dir: &Path) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                return Ok((files, problems));
            }
            return Err(format!("cannot read {}: {}", dir.display(), e));
        }
    };
    for entry in rd {
        let entry = entry.map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;
        let p = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            problems.push(format!("{}: unexpected directory", fname));
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) == Some("xml") {
            files.push(p);
        } else {
            problems.push(format!("{}: unexpected non-XML file", fname));
        }
    }
    files.sort();
    Ok((files, problems))
}
