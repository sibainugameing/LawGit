//! User-authored rule documents (`rules/<UUID>.xml`, root element `rule`).
//!
//! ```xml
//! <rule>
//!     <identifiers><uuid>...</uuid></identifiers>
//!     <metadata><name>..</name><category>..</category><created>..</created></metadata>
//!     <body format="markdown"><![CDATA[
//! Markdown text
//! ]]></body>
//! </rule>
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::repo::{self, Report, Repo};
use crate::util::{
    clean_single_line, content_version, file_name_of, is_xml_char, now_utc_iso, write_atomic,
    write_new,
};
use crate::uuid;
use crate::xml::{self, escape_text, Element};

pub const DEFAULT_CATEGORY: &str = "general";

#[derive(Debug, Clone)]
pub struct RuleDoc {
    pub uuid: String,
    pub name: String,
    pub category: String,
    pub created: String,
    pub body: String,
    pub path: PathBuf,
    /// Hash of the file content; changes whenever the file changes.
    pub version: String,
}

/// Normalize line endings, reject characters illegal in XML, trim surrounding blank lines.
pub fn normalize_body(body: &str) -> Result<String, String> {
    let b = body.replace("\r\n", "\n").replace('\r', "\n");
    if b.chars().any(|c| !is_xml_char(c)) {
        return Err("invalid params: body contains characters that are not allowed in XML".to_string());
    }
    Ok(b.trim_matches('\n').to_string())
}

pub fn render(uuid_s: &str, name: &str, category: &str, created: &str, body: &str) -> String {
    let cdata = body.replace("]]>", "]]]]><![CDATA[>");
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rule>\n    <identifiers>\n");
    s.push_str(&format!("        <uuid>{}</uuid>\n", uuid_s));
    s.push_str("    </identifiers>\n\n    <metadata>\n");
    s.push_str(&format!("        <name>{}</name>\n", escape_text(name)));
    s.push_str(&format!("        <category>{}</category>\n", escape_text(category)));
    if !created.is_empty() {
        s.push_str(&format!("        <created>{}</created>\n", escape_text(created)));
    }
    s.push_str("    </metadata>\n\n");
    s.push_str(&format!("    <body format=\"markdown\"><![CDATA[\n{}\n]]></body>\n</rule>\n", cdata));
    s
}

fn child_text(parent: &Element, name: &str) -> Option<String> {
    parent.child(name).map(|e| e.text().trim().to_string())
}

fn strip_one_newline_each_side(t: &str) -> String {
    let a = t.strip_prefix('\n').unwrap_or(t);
    let b = a.strip_suffix('\n').unwrap_or(a);
    b.to_string()
}

pub fn load_file(path: &Path) -> Result<RuleDoc, Vec<String>> {
    let src = fs::read_to_string(path)
        .map_err(|e| vec![format!("cannot read file as UTF-8 text: {}", e)])?;
    let mut errs: Vec<String> = Vec::new();
    if src.contains('\r') {
        errs.push("line endings must be LF (CR found)".to_string());
    }
    let root = match xml::parse(&src) {
        Ok(r) => r,
        Err(e) => {
            errs.push(format!("XML parse error: {}", e));
            return Err(errs);
        }
    };
    if root.name != "rule" {
        errs.push(format!("root element must be <rule>, found <{}>", root.name));
        return Err(errs);
    }

    let mut uuid_v = String::new();
    let mut name = String::new();
    let mut category = String::new();
    let mut created = String::new();
    let mut body = String::new();

    match root.child("identifiers").and_then(|i| child_text(i, "uuid")) {
        None => errs.push("missing required element <identifiers>/<uuid>".to_string()),
        Some(u) => {
            if !uuid::is_valid_v4(&u) {
                errs.push(format!("UUID is not a valid lowercase UUID v4: '{}'", u));
            }
            uuid_v = u;
        }
    }

    match root.child("metadata") {
        None => errs.push("missing required element <metadata>".to_string()),
        Some(meta) => {
            match child_text(meta, "name") {
                Some(n) if !n.is_empty() => name = n,
                _ => errs.push("<metadata>/<name> is required and must not be empty".to_string()),
            }
            match child_text(meta, "category") {
                Some(c) if !c.is_empty() => category = c,
                _ => errs.push("<metadata>/<category> is required and must not be empty".to_string()),
            }
            if let Some(c) = child_text(meta, "created") {
                created = c;
            }
        }
    }

    match root.child("body") {
        None => errs.push("missing required element <body>".to_string()),
        Some(b) => {
            body = strip_one_newline_each_side(&b.text());
        }
    }

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext != "xml" {
        errs.push("file extension must be .xml".to_string());
    }
    if !uuid_v.is_empty() && stem != uuid_v.as_str() {
        errs.push(format!("file name '{}' does not match UUID '{}'", stem, uuid_v));
    }

    if errs.is_empty() {
        Ok(RuleDoc {
            uuid: uuid_v,
            name,
            category,
            created,
            body,
            path: path.to_path_buf(),
            version: content_version(src.as_bytes()),
        })
    } else {
        Err(errs)
    }
}

pub fn validate_all(repo: &Repo) -> Result<Report<RuleDoc>, String> {
    repo.require_initialized()?;
    let (files, mut errors) = repo::xml_files(&repo.rules_dir())?;
    let mut items: Vec<RuleDoc> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in files {
        let fname = file_name_of(&f);
        match load_file(&f) {
            Ok(doc) => {
                if let Some(prev) = seen.get(&doc.uuid) {
                    errors.push(format!("{}: duplicate UUID (also used by {})", fname, prev));
                } else {
                    seen.insert(doc.uuid.clone(), fname.clone());
                    items.push(doc);
                }
            }
            Err(es) => {
                for e in es {
                    errors.push(format!("{}: {}", fname, e));
                }
            }
        }
    }
    Ok(Report { items, errors })
}

pub fn path_for(repo: &Repo, uuid_s: &str) -> Result<PathBuf, String> {
    repo.require_initialized()?;
    if !uuid::is_valid_v4(uuid_s) {
        return Err(format!(
            "invalid params: '{}' is not a valid lowercase UUID v4",
            uuid_s
        ));
    }
    let p = repo.rules_dir().join(format!("{}.xml", uuid_s));
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("not found: rule {}", uuid_s))
    }
}

pub fn get(repo: &Repo, uuid_s: &str) -> Result<RuleDoc, String> {
    let p = path_for(repo, uuid_s)?;
    load_file(&p)
        .map_err(|e| format!("invalid: rule document {} is not valid: {}", uuid_s, e.join("; ")))
}

pub fn create(repo: &Repo, name: &str, category: &str, body: &str) -> Result<RuleDoc, String> {
    repo.require_initialized()?;
    let name = clean_single_line("name", name)?;
    let category = if category.trim().is_empty() {
        DEFAULT_CATEGORY.to_string()
    } else {
        clean_single_line("category", category)?
    };
    let body = normalize_body(body)?;
    let u = repo.fresh_uuid()?;
    fs::create_dir_all(repo.rules_dir())
        .map_err(|e| format!("cannot create {}: {}", repo.rules_dir().display(), e))?;
    let path = repo.rules_dir().join(format!("{}.xml", u));
    let text = render(&u, &name, &category, &now_utc_iso(), &body);
    write_new(&path, &text)?;
    load_file(&path).map_err(|e| format!("internal: generated rule is invalid: {}", e.join("; ")))
}

fn check_version(current: &RuleDoc, expected: Option<&str>) -> Result<(), String> {
    if let Some(v) = expected {
        if v != current.version {
            return Err(format!(
                "conflict: rule {} was modified by someone else (current version {}, yours {})",
                current.uuid, current.version, v
            ));
        }
    }
    Ok(())
}

/// Update name / category / body of an existing rule. The UUID and creation time are kept.
pub fn update(
    repo: &Repo,
    uuid_s: &str,
    name: Option<&str>,
    category: Option<&str>,
    body: Option<&str>,
) -> Result<RuleDoc, String> {
    update_checked(repo, uuid_s, name, category, body, None)
}

/// Like `update`, but fails with `conflict: ...` if the stored version differs from `expected_version`.
pub fn update_checked(
    repo: &Repo,
    uuid_s: &str,
    name: Option<&str>,
    category: Option<&str>,
    body: Option<&str>,
    expected_version: Option<&str>,
) -> Result<RuleDoc, String> {
    if name.is_none() && category.is_none() && body.is_none() {
        return Err("invalid params: nothing to update (give name, category or body)".to_string());
    }
    let current = get(repo, uuid_s)?;
    check_version(&current, expected_version)?;
    let new_name = match name {
        Some(n) => clean_single_line("name", n)?,
        None => current.name.clone(),
    };
    let new_category = match category {
        Some(c) => clean_single_line("category", c)?,
        None => current.category.clone(),
    };
    let new_body = match body {
        Some(b) => normalize_body(b)?,
        None => current.body.clone(),
    };
    let text = render(&current.uuid, &new_name, &new_category, &current.created, &new_body);
    write_atomic(&current.path, &text)?;
    get(repo, uuid_s)
}

pub fn delete(repo: &Repo, uuid_s: &str) -> Result<(), String> {
    delete_checked(repo, uuid_s, None).map(|_| ())
}

/// Delete a rule (optionally only if its version matches). Returns the rule's display name.
pub fn delete_checked(repo: &Repo, uuid_s: &str, expected_version: Option<&str>) -> Result<String, String> {
    let p = path_for(repo, uuid_s)?;
    let doc = load_file(&p).ok();
    if expected_version.is_some() {
        match &doc {
            Some(d) => check_version(d, expected_version)?,
            None => {
                return Err("invalid: rule is not valid, so its version cannot be checked".to_string());
            }
        }
    }
    fs::remove_file(&p).map_err(|e| format!("internal: cannot delete {}: {}", p.display(), e))?;
    Ok(doc.map(|d| d.name).unwrap_or_else(|| uuid_s.to_string()))
}

/// Filter by exact category and/or case-insensitive substring over name, category and body.
pub fn filter(items: Vec<RuleDoc>, category: Option<&str>, query: Option<&str>) -> Vec<RuleDoc> {
    let q = query.map(|s| s.to_lowercase());
    items
        .into_iter()
        .filter(|r| match category {
            Some(c) => r.category == c,
            None => true,
        })
        .filter(|r| match &q {
            Some(q) => {
                r.name.to_lowercase().contains(q.as_str())
                    || r.category.to_lowercase().contains(q.as_str())
                    || r.body.to_lowercase().contains(q.as_str())
            }
            None => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lawgit-test-rule-{}", uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let u = uuid::new_v4();
        let p = dir.join(format!("{}.xml", u));
        let body = "# 見出し\n\n- a < b && c > d\n- cdata end: ]]> ok\n\n.\n";
        let nb = normalize_body(body).unwrap();
        fs::write(&p, render(&u, "社内規則", "運用", "2026-01-01T00:00:00Z", &nb)).unwrap();
        let doc = load_file(&p).unwrap();
        assert_eq!(doc.name, "社内規則");
        assert_eq!(doc.category, "運用");
        assert_eq!(doc.body, nb);
        assert_eq!(doc.uuid, u);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_body_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lawgit-test-rule2-{}", uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let u = uuid::new_v4();
        let p = dir.join(format!("{}.xml", u));
        fs::write(&p, render(&u, "n", "c", "", "")).unwrap();
        let doc = load_file(&p).unwrap();
        assert_eq!(doc.body, "");
        let _ = fs::remove_dir_all(&dir);
    }
}
