//! UUID v4 generation and validation (standard library only).

use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

fn random_bytes() -> [u8; 16] {
    let mut b = [0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(&mut b).is_ok() {
            return b;
        }
    }
    // Fallback (e.g. Windows): RandomState is seeded from the OS RNG.
    let nanos: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let state = RandomState::new();
    for chunk in 0..2usize {
        let mut h = state.build_hasher();
        h.write_u64(nanos);
        h.write_usize(chunk);
        h.write_u32(std::process::id());
        let v = h.finish().to_le_bytes();
        b[chunk * 8..chunk * 8 + 8].copy_from_slice(&v);
    }
    b
}

/// Generate a lowercase UUID v4.
pub fn new_v4() -> String {
    let mut b = random_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut s = String::with_capacity(36);
    for (i, byte) in b.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push_str(&format!("{:02x}", byte));
    }
    s
}

/// Canonical lowercase UUID v4: `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`, y in 8,9,a,b.
pub fn is_valid_v4(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    if c.len() != 36 {
        return false;
    }
    for (i, ch) in c.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => *ch == '-',
            14 => *ch == '4',
            19 => matches!(*ch, '8' | '9' | 'a' | 'b'),
            _ => ch.is_ascii_digit() || ('a'..='f').contains(ch),
        };
        if !ok {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_is_valid_and_unique() {
        let a = new_v4();
        let b = new_v4();
        assert!(is_valid_v4(&a));
        assert!(is_valid_v4(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn validation() {
        assert!(is_valid_v4("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_v4("550E8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_v4("550e8400-e29b-11d4-a716-446655440000"));
        assert!(!is_valid_v4("550e8400-e29b-41d4-c716-446655440000"));
        assert!(!is_valid_v4("../etc/passwd"));
        assert!(!is_valid_v4(""));
    }
}
