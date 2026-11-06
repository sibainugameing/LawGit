//! Combined validation of `laws/` and `rules/`.

use std::collections::HashSet;

use crate::law;
use crate::repo::Repo;
use crate::rule;

pub struct CheckReport {
    pub laws: usize,
    pub rules: usize,
    pub errors: Vec<String>,
}

pub fn check_all(repo: &Repo) -> Result<CheckReport, String> {
    let l = law::validate_all(repo)?;
    let r = rule::validate_all(repo)?;
    let mut errors: Vec<String> = Vec::new();
    for e in &l.errors {
        errors.push(format!("laws/{}", e));
    }
    for e in &r.errors {
        errors.push(format!("rules/{}", e));
    }
    let law_ids: HashSet<&str> = l.items.iter().map(|x| x.uuid.as_str()).collect();
    for x in &r.items {
        if law_ids.contains(x.uuid.as_str()) {
            errors.push(format!("rules/{}.xml: UUID is also used by a law document", x.uuid));
        }
    }
    Ok(CheckReport {
        laws: l.items.len(),
        rules: r.items.len(),
        errors,
    })
}
