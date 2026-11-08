//! Full-text search over legal documents and rules.

use std::fs;

use crate::json::Json;
use crate::law;
use crate::repo::Repo;
use crate::rule;
use crate::xml;

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// A short excerpt around the first case-insensitive match of `needle_lc` (already lowercase).
pub fn snippet_of(text: &str, needle_lc: &str) -> Option<String> {
    let lc: Vec<char> = text.to_lowercase().chars().collect();
    let orig: Vec<char> = text.chars().collect();
    let needle: Vec<char> = needle_lc.chars().collect();
    if needle.is_empty() || lc.len() < needle.len() {
        return None;
    }
    let pos = (0..=lc.len() - needle.len()).find(|&i| lc[i..i + needle.len()] == needle[..])?;
    let src: &Vec<char> = if lc.len() == orig.len() { &orig } else { &lc };
    let start = pos.saturating_sub(24);
    let end = (pos + needle.len() + 24).min(src.len());
    Some(src[start..end].iter().collect())
}

fn law_body_text(path: &std::path::Path) -> Option<String> {
    let src = fs::read_to_string(path).ok()?;
    let root = xml::parse(&src).ok()?;
    let body = root.child("body")?;
    Some(collapse(&body.text()))
}

fn contains_lc(hay: &str, needle_lc: &str) -> bool {
    hay.to_lowercase().contains(needle_lc)
}

fn hit(kind: &str, uuid_s: &str, name: &str, matched: &[&str], snippet: String) -> Json {
    Json::obj(vec![
        ("kind", Json::str(kind)),
        ("uuid", Json::str(uuid_s)),
        ("name", Json::str(name)),
        ("matched", Json::Arr(matched.iter().map(|m| Json::str(m)).collect())),
        ("snippet", Json::Str(snippet)),
    ])
}

/// Search names, numbers, categories and body text. Returns `{query, hits:[...], truncated}`.
pub fn search(repo: &Repo, query: &str, limit: usize) -> Result<Json, String> {
    let q = collapse(query);
    if q.is_empty() {
        return Err("invalid params: 'q' must not be empty".to_string());
    }
    let q_lc = q.to_lowercase();
    let laws = law::validate_all(repo)?;
    let rules = rule::validate_all(repo)?;
    let mut hits: Vec<Json> = Vec::new();
    let mut truncated = false;

    for l in &laws.items {
        let mut matched: Vec<&str> = Vec::new();
        let mut snippet = String::new();
        if contains_lc(&l.name, &q_lc) {
            matched.push("name");
        }
        if l.law_number.as_deref().map(|s| contains_lc(s, &q_lc)).unwrap_or(false) {
            matched.push("law_number");
        }
        if l.e_gov_id.as_deref().map(|s| contains_lc(s, &q_lc)).unwrap_or(false) {
            matched.push("e_gov_id");
        }
        if let Some(body) = law_body_text(&l.path) {
            if let Some(s) = snippet_of(&body, &q_lc) {
                matched.push("body");
                snippet = s;
            }
        }
        if !matched.is_empty() {
            if hits.len() >= limit {
                truncated = true;
                break;
            }
            hits.push(hit("law", &l.uuid, &l.name, &matched, snippet));
        }
    }

    if !truncated {
        for r in &rules.items {
            let mut matched: Vec<&str> = Vec::new();
            let mut snippet = String::new();
            if contains_lc(&r.name, &q_lc) {
                matched.push("name");
            }
            if contains_lc(&r.category, &q_lc) {
                matched.push("category");
            }
            if let Some(s) = snippet_of(&collapse(&r.body), &q_lc) {
                matched.push("body");
                snippet = s;
            }
            if !matched.is_empty() {
                if hits.len() >= limit {
                    truncated = true;
                    break;
                }
                hits.push(hit("rule", &r.uuid, &r.name, &matched, snippet));
            }
        }
    }

    Ok(Json::obj(vec![
        ("query", Json::Str(q)),
        ("hits", Json::Arr(hits)),
        ("truncated", Json::Bool(truncated)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets() {
        let s = snippet_of("この法律は、道路における危険を防止し、交通の安全を図る", "交通の安全").unwrap();
        assert!(s.contains("交通の安全"));
        assert!(snippet_of("abc", "x").is_none());
        assert!(snippet_of("Hello World", "world").unwrap().contains("World"));
        assert!(snippet_of("", "a").is_none());
    }

    #[test]
    fn end_to_end_search() {
        let dir = std::env::temp_dir().join(format!("lawgit-test-search-{}", crate::uuid::new_v4()));
        let repo = Repo::new(&dir);
        repo.init().unwrap();
        rule::create(&repo, "休暇規則", "人事", "年次有給休暇は年5日").unwrap();
        rule::create(&repo, "経費規則", "経理", "領収書を保管する").unwrap();
        let r = search(&repo, "有給", 10).unwrap();
        match r.get("hits") {
            Some(Json::Arr(h)) => assert_eq!(h.len(), 1),
            _ => panic!("no hits array"),
        }
        assert!(search(&repo, "  ", 10).is_err());
        let r = search(&repo, "規則", 1).unwrap();
        assert_eq!(r.get("truncated"), Some(&Json::Bool(true)));
        let _ = fs::remove_dir_all(&dir);
    }
}
