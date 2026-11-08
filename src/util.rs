//! Small shared helpers (standard library only).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_utc_iso() -> String {
    let secs: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// True if `c` is allowed in an XML 1.0 document.
pub fn is_xml_char(c: char) -> bool {
    c == '\t'
        || c == '\n'
        || c == '\r'
        || (c >= '\u{20}' && c <= '\u{d7ff}')
        || (c >= '\u{e000}' && c <= '\u{fffd}')
        || c >= '\u{10000}'
}

/// Trim and require a non-empty single-line value without control characters.
pub fn clean_single_line(label: &str, value: &str) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("invalid params: {} must not be empty", label));
    }
    if v.chars().any(|c| c.is_control() || !is_xml_char(c)) {
        return Err(format!(
            "invalid params: {} must be a single line without control characters",
            label
        ));
    }
    Ok(v.to_string())
}

/// Like `clean_single_line`, but empty / missing input yields `None`.
pub fn clean_optional(label: &str, value: Option<&str>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(v) => {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                clean_single_line(label, v).map(Some)
            }
        }
    }
}

/// Create a new file; fails if it already exists.
pub fn write_new(path: &Path, content: &str) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create {}: {}", path.display(), e))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    f.flush()
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

/// Replace a file's content via a temporary file and rename.
pub fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    fs::write(&tmp, content).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {}", path.display(), e)
    })
}

/// Stable 64-bit FNV-1a hash of the bytes, as 16 hex digits (used as an edit "version").
pub fn content_version(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode `%XX` escapes and `+` (query-string style).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("%E6%B3%95"), "法");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn clean() {
        assert!(clean_single_line("x", "  ").is_err());
        assert!(clean_single_line("x", "a\nb").is_err());
        assert_eq!(clean_single_line("x", " a ").unwrap(), "a");
        assert_eq!(clean_optional("x", Some(" ")).unwrap(), None);
    }

    #[test]
    fn version_is_stable_and_sensitive() {
        assert_eq!(content_version(b""), "cbf29ce484222325");
        assert_eq!(content_version(b"a"), content_version(b"a"));
        assert_ne!(content_version(b"a"), content_version(b"b"));
    }

    #[test]
    fn date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19723), (2024, 1, 1));
    }
}
