pub mod rules;
pub mod amendments;
pub use amendments::{amend_law, Amendment, AmendmentChange};
pub use rules::{create_rule, list_rules, prompt_and_create_rule, read_rule, validate_rule_repository, NewRule, RuleSummary};

use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NewLaw {
    pub name: String,
    pub law_type: String,
    pub law_number: Option<String>,
    pub e_gov_id: Option<String>,
    pub promulgated_date: Option<String>,
    pub effective_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LawSummary {
    pub uuid: String,
    pub name: String,
    pub law_type: String,
    pub law_number: Option<String>,
    pub e_gov_id: Option<String>,
    pub path: PathBuf,
}

/// Initialize a directory as a Git-backed law repository.
pub fn init_repository(repo: &Path) -> Result<()> {
    fs::create_dir_all(repo)
        .with_context(|| format!("cannot create repository directory: {}", repo.display()))?;

    if !repo.join(".git").exists() {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("init")
            .output()
            .context("Git is required but could not be executed")?;

        if !output.status.success() {
            bail!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }

    fs::create_dir_all(repo.join("laws"))
        .with_context(|| format!("cannot create laws directory in {}", repo.display()))?;
    fs::create_dir_all(repo.join("rules"))
        .with_context(|| format!("cannot create rules directory in {}", repo.display()))?;

    Ok(())
}

/// Create a new law XML file with a generated UUID v4.
pub fn create_law(repo: &Path, input: &NewLaw) -> Result<LawSummary> {
    ensure_git_repository(repo)?;
    let laws_dir = repo.join("laws");
    fs::create_dir_all(&laws_dir)
        .with_context(|| format!("cannot create {}", laws_dir.display()))?;

    validate_xml_text(&input.name, "name")?;
    validate_xml_text(&input.law_type, "type")?;

    let name = input.name.trim();
    let law_type = input.law_type.trim();
    if name.is_empty() {
        bail!("law name must not be empty");
    }
    if law_type.is_empty() {
        bail!("law type must not be empty");
    }

    let law_number = normalize_optional(input.law_number.as_deref());
    let e_gov_id = normalize_optional(input.e_gov_id.as_deref());
    let promulgated_date = normalize_optional(input.promulgated_date.as_deref());
    let effective_date = normalize_optional(input.effective_date.as_deref());

    if let Some(value) = &promulgated_date {
        validate_iso_date(value, "promulgated-date")?;
    }
    if let Some(value) = &effective_date {
        validate_iso_date(value, "effective-date")?;
    }

    if let Some(value) = &law_number {
        validate_xml_text(value, "law-number")?;
    }
    if let Some(value) = &e_gov_id {
        validate_xml_text(value, "e-gov-id")?;
        if value.is_empty() {
            bail!("e-Gov Legal ID must not be empty when provided");
        }
    }

    let uuid = Uuid::new_v4().to_string();
    let path = laws_dir.join(format!("{uuid}.xml"));

    let law_number_xml = law_number
        .as_deref()
        .map(|value| format!("        <law-number>{}</law-number>\n", escape_xml(value)))
        .unwrap_or_default();

    let e_gov_id_xml = e_gov_id
        .as_deref()
        .map(|value| format!("        <e-gov-id>{}</e-gov-id>\n", escape_xml(value)))
        .unwrap_or_default();

    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<law>\n\
    <identifiers>\n\
        <uuid>{}</uuid>\n\
{}\
    </identifiers>\n\
\n\
    <metadata>\n\
        <name>{}</name>\n\
{}\
        <type>{}</type>\n\
    </metadata>\n\
\n\
    <body />\n\
</law>\n",
        uuid,
        e_gov_id_xml,
        escape_xml(name),
        law_number_xml,
        escape_xml(law_type),
    );

    if let Some(value) = &promulgated_date {
        xml = xml.replace(
            "    </metadata>",
            &format!("        <promulgated-date>{}</promulgated-date>\n    </metadata>", escape_xml(value)),
        );
    }
    if let Some(value) = &effective_date {
        xml = xml.replace(
            "    </metadata>",
            &format!("        <effective-date>{}</effective-date>\n    </metadata>", escape_xml(value)),
        );
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("cannot create law file: {}", path.display()))?;
    file.write_all(xml.as_bytes())
        .with_context(|| format!("cannot write law file: {}", path.display()))?;

    parse_law_file(&path)
}

/// Validate every XML file in the repository's laws directory.
pub fn validate_repository(repo: &Path) -> Result<Vec<LawSummary>> {
    ensure_git_repository(repo)?;
    let laws_dir = repo.join("laws");
    if !laws_dir.is_dir() {
        bail!("laws directory does not exist: {}", laws_dir.display());
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(&laws_dir)
        .with_context(|| format!("cannot read {}", laws_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("xml")
        {
            paths.push(path);
        }
    }
    paths.sort();

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut laws = Vec::with_capacity(paths.len());

    for path in paths {
        let law = parse_law_file(&path)?;
        if let Some(previous_path) = seen.insert(law.uuid.clone(), law.path.clone()) {
            bail!(
                "duplicate lawgit UUID {} in {} and {}",
                law.uuid,
                previous_path.display(),
                law.path.display()
            );
        }
        laws.push(law);
    }

    Ok(laws)
}

/// Return validated law metadata, ordered by UUID.
pub fn list_laws(repo: &Path) -> Result<Vec<LawSummary>> {
    let mut laws = validate_repository(repo)?;
    laws.sort_by(|left, right| left.uuid.cmp(&right.uuid));
    Ok(laws)
}

/// Read the complete XML document identified by a lawgit UUID.
pub fn read_law(repo: &Path, uuid: &str) -> Result<(LawSummary, String)> {
    let laws = validate_repository(repo)?;
    let law = laws
        .into_iter()
        .find(|law| law.uuid == uuid)
        .with_context(|| format!("law not found: {uuid}"))?;
    let content = fs::read_to_string(&law.path)
        .with_context(|| format!("cannot read {}", law.path.display()))?;
    Ok((law, content))
}

fn ensure_git_repository(repo: &Path) -> Result<()> {
    if !repo.join(".git").exists() {
        bail!(
            "{} is not a Git repository; run lawgit init first",
            repo.display()
        );
    }
    Ok(())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned)
}

fn validate_xml_text(value: &str, field: &str) -> Result<()> {
    for character in value.chars() {
        let code = character as u32;
        let valid = matches!(code, 0x9 | 0xA | 0xD)
            || (0x20..=0xD7FF).contains(&code)
            || (0xE000..=0xFFFD).contains(&code)
            || (0x10000..=0x10FFFF).contains(&code);
        if !valid {
            bail!("{field} contains a character forbidden by XML 1.0");
        }
    }
    Ok(())
}

pub(crate) fn validate_iso_date(value: &str, field: &str) -> Result<()> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        bail!("{field} must use YYYY-MM-DD format: {value}");
    }
    let year: u32 = parts[0].parse().with_context(|| format!("invalid {field}: {value}"))?;
    let month: u32 = parts[1].parse().with_context(|| format!("invalid {field}: {value}"))?;
    let day: u32 = parts[2].parse().with_context(|| format!("invalid {field}: {value}"))?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > days_in_month {
        bail!("invalid {field} calendar date: {value}");
    }
    Ok(())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn one_child<'a, 'input>(parent: Node<'a, 'input>, name: &str) -> Result<Node<'a, 'input>> {
    let mut matches = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name);

    let node = matches
        .next()
        .with_context(|| format!("missing required element <{name}>"))?;

    if matches.next().is_some() {
        bail!("element <{name}> must appear only once");
    }
    if node.tag_name().namespace().is_some() {
        bail!("namespaced element <{name}> is not supported by the current specification");
    }

    Ok(node)
}

fn optional_child<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>> {
    let mut matches = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name);

    let node = matches.next();
    if matches.next().is_some() {
        bail!("element <{name}> must appear only once");
    }
    if let Some(node) = node {
        if node.tag_name().namespace().is_some() {
            bail!("namespaced element <{name}> is not supported by the current specification");
        }
    }

    Ok(node)
}

fn simple_text(node: Node<'_, '_>, name: &str) -> Result<String> {
    if node.children().any(|child| child.is_element()) {
        bail!("element <{name}> must contain text, not nested elements");
    }
    Ok(node.text().unwrap_or("").trim().to_owned())
}

fn required_text(parent: Node<'_, '_>, name: &str) -> Result<String> {
    let value = simple_text(one_child(parent, name)?, name)?;
    if value.is_empty() {
        bail!("element <{name}> must not be empty");
    }
    Ok(value)
}

fn optional_text(parent: Node<'_, '_>, name: &str) -> Result<Option<String>> {
    optional_child(parent, name)?
        .map(|node| simple_text(node, name))
        .transpose()
}

fn parse_law_file(path: &Path) -> Result<LawSummary> {
    let xml = fs::read_to_string(path)
        .with_context(|| format!("cannot read XML file: {}", path.display()))?;
    let document = Document::parse(&xml)
        .with_context(|| format!("invalid XML in {}", path.display()))?;
    let root = document.root_element();

    if root.tag_name().name() != "law" || root.tag_name().namespace().is_some() {
        bail!("{}: root element must be <law>", path.display());
    }

    let identifiers = one_child(root, "identifiers")?;
    let metadata = one_child(root, "metadata")?;
    let body = one_child(root, "body")?;

    let uuid = required_text(identifiers, "uuid")?;
    let parsed_uuid =
        Uuid::parse_str(&uuid).with_context(|| format!("invalid UUID: {uuid}"))?;
    let bytes = parsed_uuid.as_bytes();

    if bytes[6] >> 4 != 4 || bytes[8] & 0b1100_0000 != 0b1000_0000 {
        bail!("UUID must use the RFC-compatible UUID v4 format: {uuid}");
    }
    if parsed_uuid.to_string() != uuid {
        bail!("UUID must use lowercase canonical hyphenated format: {uuid}");
    }

    let expected_filename = format!("{uuid}.xml");
    let actual_filename = path.file_name().and_then(|name| name.to_str());
    if actual_filename != Some(expected_filename.as_str()) {
        bail!(
            "{}: filename must match the UUID ({expected_filename})",
            path.display()
        );
    }

    let name = required_text(metadata, "name")?;
    let law_type = required_text(metadata, "type")?;
    let law_number = optional_text(metadata, "law-number")?;
    let e_gov_id = optional_text(identifiers, "e-gov-id")?;
    if let Some(value) = optional_text(metadata, "promulgated-date")? {
        validate_iso_date(&value, "promulgated-date")?;
    }
    if let Some(value) = optional_text(metadata, "effective-date")? {
        validate_iso_date(&value, "effective-date")?;
    }

    if e_gov_id.as_deref().is_some_and(str::is_empty) {
        bail!("e-Gov Legal ID must not be empty when present: {}", path.display());
    }

    // The specification requires the legal body to be represented as XML structure,
    // not as one unstructured text value. Whitespace used for formatting is allowed.
    if body.children().any(|child| {
        child.is_text() && !child.text().unwrap_or("").trim().is_empty()
    }) {
        bail!(
            "{}: <body> must contain structured XML elements, not plain text",
            path.display()
        );
    }

    Ok(LawSummary {
        uuid,
        name,
        law_type,
        law_number,
        e_gov_id,
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn test_repository() -> PathBuf {
        let path = env::temp_dir().join(format!("lawgit-test-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::create_dir_all(path.join("laws")).unwrap();
        path
    }

    #[test]
    fn creates_and_validates_a_law() {
        let repo = test_repository();
        let input = NewLaw {
            name: "Test & Sample Act".to_owned(),
            law_type: "法律".to_owned(),
            law_number: Some("令和八年法律第一号".to_owned()),
            e_gov_id: Some("415AC0000000057".to_owned()),
            promulgated_date: Some("2026-01-15".to_owned()),
            effective_date: Some("2026-04-01".to_owned()),
        };

        let created = create_law(&repo, &input).unwrap();
        assert_eq!(created.name, "Test & Sample Act");
        assert!(created.path.ends_with(format!("{}.xml", created.uuid)));

        let xml = fs::read_to_string(&created.path).unwrap();
        assert!(xml.contains("Test &amp; Sample Act"));
        assert!(xml.contains("<promulgated-date>2026-01-15</promulgated-date>"));
        assert!(xml.contains("<effective-date>2026-04-01</effective-date>"));

        let validated = validate_repository(&repo).unwrap();
        assert_eq!(validated.len(), 1);
        assert_eq!(validated[0].uuid, created.uuid);

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn rejects_non_structured_body_text() {
        let repo = test_repository();
        let uuid = Uuid::new_v4().to_string();
        let path = repo.join("laws").join(format!("{uuid}.xml"));
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<law><identifiers><uuid>{uuid}</uuid></identifiers>\
<metadata><name>Test</name><type>法律</type></metadata>\
<body>plain text</body></law>"
        );
        fs::write(&path, xml).unwrap();

        assert!(validate_repository(&repo).is_err());
        fs::remove_dir_all(repo).unwrap();
    }
}
