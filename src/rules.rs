use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Input for a user-authored rule document.
#[derive(Debug, Clone)]
pub struct NewRule {
    pub name: String,
    pub rule_type: String,
    pub body: String,
}

/// Validated metadata for a user-authored rule document.
#[derive(Debug, Clone)]
pub struct RuleSummary {
    pub uuid: String,
    pub name: String,
    pub rule_type: String,
    pub path: PathBuf,
}

/// Interactive authoring mode.
/// Enter a single dot (.) on its own line to finish the body.
/// Enter two dots (..) to insert a literal dot line.
pub fn prompt_and_create_rule(repo: &Path) -> Result<RuleSummary> {
    let name = prompt_required("Rule name: ")?;
    let rule_type = prompt_required("Rule type/category: ")?;

    println!("Body (Markdown is allowed).");
    println!("Finish with a single '.' on its own line; '..' inserts a literal dot.");

    let stdin = io::stdin();
    let mut body = String::new();
    loop {
        let mut line = String::new();
        let read = stdin
            .read_line(&mut line)
            .context("failed to read rule body from standard input")?;
        if read == 0 {
            bail!("input ended before the rule body terminator (.) was entered");
        }

        let content = line.trim_end_matches(&['\r', '\n'][..]);
        if content == "." {
            break;
        }

        if content == ".." {
            body.push('.');
            body.push('\n');
        } else {
            body.push_str(content);
            body.push('\n');
        }
    }

    create_rule(
        repo,
        &NewRule {
            name,
            rule_type,
            body,
        },
    )
}

fn prompt_required(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("failed to flush standard output")?;

    let mut value = String::new();
    let read = io::stdin()
        .read_line(&mut value)
        .context("failed to read input from standard input")?;
    if read == 0 {
        bail!("input ended while waiting for {label}");
    }

    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(value)
}

/// Create a rule XML file using a generated UUID v4.
pub fn create_rule(repo: &Path, input: &NewRule) -> Result<RuleSummary> {
    ensure_git_repository(repo)?;

    let name = input.name.trim();
    let rule_type = input.rule_type.trim();
    if name.is_empty() {
        bail!("rule name must not be empty");
    }
    if rule_type.is_empty() {
        bail!("rule type/category must not be empty");
    }

    validate_xml_text(name, "name")?;
    validate_xml_text(rule_type, "type")?;
    validate_xml_text(&input.body, "body")?;

    let rules_dir = repo.join("rules");
    fs::create_dir_all(&rules_dir)
        .with_context(|| format!("cannot create {}", rules_dir.display()))?;

    let uuid = Uuid::new_v4().to_string();
    let path = rules_dir.join(format!("{uuid}.xml"));
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<rule>\n\
    <identifiers>\n\
        <uuid>{uuid}</uuid>\n\
    </identifiers>\n\
\n\
    <metadata>\n\
        <name>{}</name>\n\
        <type>{}</type>\n\
    </metadata>\n\
\n\
    <body>{}</body>\n\
</rule>\n",
        escape_xml(name),
        escape_xml(rule_type),
        escape_xml(&input.body),
    );

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("cannot create rule file: {}", path.display()))?;
    file.write_all(xml.as_bytes())
        .with_context(|| format!("cannot write rule file: {}", path.display()))?;

    parse_rule_file(&path)
}

/// Validate every XML document in the repository's rules directory.
pub fn validate_rule_repository(repo: &Path) -> Result<Vec<RuleSummary>> {
    ensure_git_repository(repo)?;

    let rules_dir = repo.join("rules");
    if !rules_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(&rules_dir)
        .with_context(|| format!("cannot read {}", rules_dir.display()))?
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
    let mut rules = Vec::with_capacity(paths.len());

    for path in paths {
        let rule = parse_rule_file(&path)?;
        if let Some(previous_path) = seen.insert(rule.uuid.clone(), rule.path.clone()) {
            bail!(
                "duplicate rule UUID {} in {} and {}",
                rule.uuid,
                previous_path.display(),
                rule.path.display()
            );
        }
        rules.push(rule);
    }

    Ok(rules)
}

/// List validated rule documents, ordered by UUID.
pub fn list_rules(repo: &Path) -> Result<Vec<RuleSummary>> {
    let mut rules = validate_rule_repository(repo)?;
    rules.sort_by(|left, right| left.uuid.cmp(&right.uuid));
    Ok(rules)
}

/// Read the complete XML document identified by a rule UUID.
pub fn read_rule(repo: &Path, uuid: &str) -> Result<(RuleSummary, String)> {
    let rules = validate_rule_repository(repo)?;
    let rule = rules
        .into_iter()
        .find(|rule| rule.uuid == uuid)
        .with_context(|| format!("rule not found: {uuid}"))?;
    let content = fs::read_to_string(&rule.path)
        .with_context(|| format!("cannot read {}", rule.path.display()))?;
    Ok((rule, content))
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
        bail!("namespaced element <{name}> is not supported");
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

fn parse_rule_file(path: &Path) -> Result<RuleSummary> {
    let xml = fs::read_to_string(path)
        .with_context(|| format!("cannot read XML file: {}", path.display()))?;
    let document = Document::parse(&xml)
        .with_context(|| format!("invalid XML in {}", path.display()))?;
    let root = document.root_element();

    if root.tag_name().name() != "rule" || root.tag_name().namespace().is_some() {
        bail!("{}: root element must be <rule>", path.display());
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
    let rule_type = required_text(metadata, "type")?;
    if body.children().any(|child| child.is_element()) {
        bail!(
            "{}: <body> must contain text (Markdown is allowed), not nested XML elements",
            path.display()
        );
    }

    Ok(RuleSummary {
        uuid,
        name,
        rule_type,
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn test_repository() -> PathBuf {
        let path = env::temp_dir().join(format!("lawgit-rule-test-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join(".git")).unwrap();
        path
    }

    #[test]
    fn creates_lists_and_reads_a_rule() {
        let repo = test_repository();
        let input = NewRule {
            name: "Server Rules & Guidelines".to_owned(),
            rule_type: "利用規約".to_owned(),
            body: "# Rules\n\nUse <safe> values & keep records.\n".to_owned(),
        };

        let created = create_rule(&repo, &input).unwrap();
        assert!(created.path.ends_with(format!("{}.xml", created.uuid)));
        assert_eq!(created.name, input.name);

        let xml = fs::read_to_string(&created.path).unwrap();
        assert!(xml.contains("Server Rules &amp; Guidelines"));
        assert!(xml.contains("&lt;safe&gt; values &amp;"));

        let listed = list_rules(&repo).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].uuid, created.uuid);

        let (read, _) = read_rule(&repo, &created.uuid).unwrap();
        assert_eq!(read.name, created.name);
        assert_eq!(read.rule_type, created.rule_type);

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn empty_rules_directory_is_valid() {
        let repo = test_repository();
        assert!(validate_rule_repository(&repo).unwrap().is_empty());
        fs::remove_dir_all(repo).unwrap();
    }
}
