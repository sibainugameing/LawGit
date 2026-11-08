//! Legal documents (`laws/<UUID>.xml`) following the lawgit standard.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::repo::{self, Report, Repo};
use crate::util::{clean_optional, clean_single_line, content_version, file_name_of, write_new};
use crate::uuid;
use crate::xml::{self, escape_text, Element};

#[derive(Debug, Clone)]
pub struct LawInfo {
    pub uuid: String,
    pub name: String,
    pub law_type: String,
    pub law_number: Option<String>,
    pub e_gov_id: Option<String>,
    pub path: PathBuf,
    /// Hash of the file content; changes whenever the file changes.
    pub version: String,
}

pub struct NewLaw {
    pub name: String,
    pub law_type: String,
    pub law_number: Option<String>,
    pub e_gov_id: Option<String>,
}

/// Render a new, empty law document (the body holds only a comment).
pub fn render(
    uuid_s: &str,
    name: &str,
    law_type: &str,
    law_number: Option<&str>,
    e_gov_id: Option<&str>,
) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<law>\n    <identifiers>\n");
    s.push_str(&format!("        <uuid>{}</uuid>\n", uuid_s));
    if let Some(id) = e_gov_id {
        s.push_str(&format!("        <e-gov-id>{}</e-gov-id>\n", escape_text(id)));
    }
    s.push_str("    </identifiers>\n\n    <metadata>\n");
    s.push_str(&format!("        <name>{}</name>\n", escape_text(name)));
    if let Some(n) = law_number {
        s.push_str(&format!("        <law-number>{}</law-number>\n", escape_text(n)));
    }
    s.push_str(&format!("        <type>{}</type>\n", escape_text(law_type)));
    s.push_str("    </metadata>\n\n    <body>\n        <!-- 法令本文: 条・項・号などを XML 要素で記述する -->\n    </body>\n</law>\n");
    s
}

fn child_text(parent: &Element, name: &str) -> Option<String> {
    parent.child(name).map(|e| e.text().trim().to_string())
}

/// Parse and validate one law file. Errors are messages without the file name.
pub fn load_file(path: &Path) -> Result<LawInfo, Vec<String>> {
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
    if root.name != "law" {
        errs.push(format!("root element must be <law>, found <{}>", root.name));
        return Err(errs);
    }

    let mut uuid_v = String::new();
    let mut egov: Option<String> = None;
    let mut name = String::new();
    let mut law_number: Option<String> = None;
    let mut law_type = String::new();

    match root.child("identifiers") {
        None => errs.push("missing required element <identifiers>".to_string()),
        Some(ids) => {
            match child_text(ids, "uuid") {
                None => errs.push("missing required element <identifiers>/<uuid>".to_string()),
                Some(u) => {
                    if !uuid::is_valid_v4(&u) {
                        errs.push(format!("UUID is not a valid lowercase UUID v4: '{}'", u));
                    }
                    uuid_v = u;
                }
            }
            if let Some(g) = child_text(ids, "e-gov-id") {
                if g.is_empty() {
                    errs.push("<e-gov-id> must not be empty when present".to_string());
                } else {
                    egov = Some(g);
                }
            }
        }
    }

    match root.child("metadata") {
        None => errs.push("missing required element <metadata>".to_string()),
        Some(meta) => {
            match child_text(meta, "name") {
                Some(n) if !n.is_empty() => name = n,
                _ => errs.push("<metadata>/<name> is required and must not be empty".to_string()),
            }
            match child_text(meta, "type") {
                Some(t) if !t.is_empty() => law_type = t,
                _ => errs.push("<metadata>/<type> is required and must not be empty".to_string()),
            }
            if let Some(n) = child_text(meta, "law-number") {
                if !n.is_empty() {
                    law_number = Some(n);
                }
            }
        }
    }

    match root.child("body") {
        None => errs.push("missing required element <body>".to_string()),
        Some(b) => {
            if b.has_direct_text() {
                errs.push(
                    "<body> must be structured with XML elements; plain text directly under <body> is not allowed"
                        .to_string(),
                );
            }
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
        Ok(LawInfo {
            uuid: uuid_v,
            name,
            law_type,
            law_number,
            e_gov_id: egov,
            path: path.to_path_buf(),
            version: content_version(src.as_bytes()),
        })
    } else {
        Err(errs)
    }
}

/// Validate every file in `laws/` including duplicate-UUID detection.
pub fn validate_all(repo: &Repo) -> Result<Report<LawInfo>, String> {
    repo.require_initialized()?;
    let (files, mut errors) = repo::xml_files(&repo.laws_dir())?;
    let mut items: Vec<LawInfo> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in files {
        let fname = file_name_of(&f);
        match load_file(&f) {
            Ok(info) => {
                if let Some(prev) = seen.get(&info.uuid) {
                    errors.push(format!("{}: duplicate UUID (also used by {})", fname, prev));
                } else {
                    seen.insert(info.uuid.clone(), fname.clone());
                    items.push(info);
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
    let p = repo.laws_dir().join(format!("{}.xml", uuid_s));
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("not found: law {}", uuid_s))
    }
}

/// Raw XML text of a law document (no validation).
pub fn read_xml(repo: &Repo, uuid_s: &str) -> Result<String, String> {
    let p = path_for(repo, uuid_s)?;
    fs::read_to_string(&p).map_err(|e| format!("internal: cannot read {}: {}", p.display(), e))
}

/// Validated info plus raw XML.
pub fn get(repo: &Repo, uuid_s: &str) -> Result<(LawInfo, String), String> {
    let p = path_for(repo, uuid_s)?;
    let xml_text =
        fs::read_to_string(&p).map_err(|e| format!("internal: cannot read {}: {}", p.display(), e))?;
    let info = load_file(&p)
        .map_err(|e| format!("invalid: law document {} is not valid: {}", uuid_s, e.join("; ")))?;
    Ok((info, xml_text))
}

pub fn create(repo: &Repo, new: &NewLaw) -> Result<LawInfo, String> {
    repo.require_initialized()?;
    let name = clean_single_line("name", &new.name)?;
    let law_type = clean_single_line("type", &new.law_type)?;
    let law_number = clean_optional("law_number", new.law_number.as_deref())?;
    let e_gov_id = clean_optional("e_gov_id", new.e_gov_id.as_deref())?;
    let u = repo.fresh_uuid()?;
    let path = repo.laws_dir().join(format!("{}.xml", u));
    let text = render(&u, &name, &law_type, law_number.as_deref(), e_gov_id.as_deref());
    write_new(&path, &text)?;
    load_file(&path).map_err(|e| format!("internal: generated document is invalid: {}", e.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_template_is_valid() {
        let dir = std::env::temp_dir().join(format!("lawgit-test-law-{}", uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let u = uuid::new_v4();
        let p = dir.join(format!("{}.xml", u));
        fs::write(&p, render(&u, "道路交通法 & 他", "法律", Some("昭和三十五年法律第百五号"), Some("335AC0000000105"))).unwrap();
        let info = load_file(&p).unwrap();
        assert_eq!(info.name, "道路交通法 & 他");
        assert_eq!(info.e_gov_id.as_deref(), Some("335AC0000000105"));
        assert_eq!(info.uuid, u);

        // wrong file name
        let bad = dir.join(format!("{}.xml", uuid::new_v4()));
        fs::copy(&p, &bad).unwrap();
        assert!(load_file(&bad).is_err());

        // plain-text body is rejected
        let u2 = uuid::new_v4();
        let p2 = dir.join(format!("{}.xml", u2));
        let t = render(&u2, "x", "法律", None, None).replace("<!-- 法令本文: 条・項・号などを XML 要素で記述する -->", "text");
        fs::write(&p2, t).unwrap();
        assert!(load_file(&p2).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
