//! A small, strict XML 1.0 parser sufficient for LawGit documents.
//!
//! Supported: XML declaration, comments, processing instructions, elements,
//! attributes, character data, CDATA sections, predefined and numeric entities.
//! Rejected: DOCTYPE / custom entities (no external entity expansion).

use crate::util::is_xml_char;

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn child(&self, name: &str) -> Option<&Element> {
        for n in &self.children {
            if let Node::Element(e) = n {
                if e.name == name {
                    return Some(e);
                }
            }
        }
        None
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.0 == name)
            .map(|a| a.1.as_str())
    }

    /// Concatenated text of this element and all descendants.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, out: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
            }
        }
    }

    /// True if there is non-whitespace character data directly inside this element.
    pub fn has_direct_text(&self) -> bool {
        self.children.iter().any(|n| match n {
            Node::Text(t) => !t.trim().is_empty(),
            Node::Element(_) => false,
        })
    }
}

pub fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn parse(src: &str) -> Result<Element, String> {
    let mut p = Parser {
        s: src.chars().collect(),
        i: 0,
    };
    if p.peek() == Some('\u{feff}') {
        p.i += 1;
    }
    if p.looking_at("<?xml") {
        p.skip_pi()?;
    }
    p.skip_misc()?;
    if p.peek() != Some('<') {
        return Err(p.err("expected root element"));
    }
    let root = p.element(0)?;
    p.skip_misc()?;
    if p.i < p.s.len() {
        return Err(p.err("unexpected content after root element"));
    }
    Ok(root)
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

fn is_name_start(c: char) -> bool {
    c == '_' || c == ':' || c.is_alphabetic()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c == '-' || c == '.' || c.is_alphanumeric()
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn err(&self, msg: &str) -> String {
        let end = if self.i < self.s.len() { self.i } else { self.s.len() };
        let line = self.s[..end].iter().filter(|c| **c == '\n').count() + 1;
        format!("line {}: {}", line, msg)
    }

    fn looking_at(&self, pat: &str) -> bool {
        let mut j = self.i;
        for c in pat.chars() {
            if self.s.get(j) != Some(&c) {
                return false;
            }
            j += 1;
        }
        true
    }

    fn find_from(&self, pat: &str) -> Option<usize> {
        let p: Vec<char> = pat.chars().collect();
        if self.s.len() < p.len() {
            return None;
        }
        let mut j = self.i;
        while j + p.len() <= self.s.len() {
            if self.s[j..j + p.len()] == p[..] {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.i += 1;
            } else {
                break;
            }
        }
        self.i > start
    }

    fn expect(&mut self, ch: char) -> Result<(), String> {
        if self.peek() == Some(ch) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", ch)))
        }
    }

    fn skip_comment(&mut self) -> Result<(), String> {
        self.i += 4; // "<!--"
        match self.find_from("-->") {
            Some(end) => {
                self.i = end + 3;
                Ok(())
            }
            None => Err(self.err("unterminated comment")),
        }
    }

    fn skip_pi(&mut self) -> Result<(), String> {
        self.i += 2; // "<?"
        match self.find_from("?>") {
            Some(end) => {
                self.i = end + 2;
                Ok(())
            }
            None => Err(self.err("unterminated processing instruction")),
        }
    }

    /// Whitespace, comments and processing instructions outside the root element.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.looking_at("<!--") {
                self.skip_comment()?;
            } else if self.looking_at("<?") {
                self.skip_pi()?;
            } else if self.looking_at("<!") {
                return Err(self.err("DOCTYPE and other declarations are not allowed"));
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        match self.peek() {
            Some(c) if is_name_start(c) => self.i += 1,
            _ => return Err(self.err("invalid name")),
        }
        while let Some(c) = self.peek() {
            if is_name_char(c) {
                self.i += 1;
            } else {
                break;
            }
        }
        Ok(self.s[start..self.i].iter().collect())
    }

    fn entity(&mut self) -> Result<char, String> {
        let start = self.i + 1;
        let mut j = start;
        while j < self.s.len() && j < start + 12 && self.s[j] != ';' {
            j += 1;
        }
        if j >= self.s.len() || self.s[j] != ';' {
            return Err(self.err("malformed entity reference"));
        }
        let body: String = self.s[start..j].iter().collect();
        let c: char = match body.as_str() {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            b if b.starts_with("#x") => u32::from_str_radix(&b[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| self.err("invalid character reference"))?,
            b if b.starts_with('#') => b[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| self.err("invalid character reference"))?,
            _ => return Err(self.err("unknown entity reference")),
        };
        if !is_xml_char(c) {
            return Err(self.err("character reference to a character not allowed in XML"));
        }
        self.i = j + 1;
        Ok(c)
    }

    fn attr_value(&mut self) -> Result<String, String> {
        let quote = match self.peek() {
            Some(q) if q == '"' || q == '\'' => q,
            _ => return Err(self.err("attribute value must be quoted")),
        };
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated attribute value")),
                Some(c) if c == quote => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('<') => return Err(self.err("'<' not allowed in attribute value")),
                Some('&') => {
                    let c = self.entity()?;
                    out.push(c);
                }
                Some(c) => {
                    if !is_xml_char(c) {
                        return Err(self.err("invalid character in attribute value"));
                    }
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn element(&mut self, depth: usize) -> Result<Element, String> {
        if depth > 256 {
            return Err(self.err("elements nested too deeply"));
        }
        self.expect('<')?;
        let name = self.name()?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let had_ws = self.skip_ws();
            match self.peek() {
                Some('>') => {
                    self.i += 1;
                    break;
                }
                Some('/') => {
                    self.i += 1;
                    self.expect('>')?;
                    return Ok(Element {
                        name,
                        attrs,
                        children: Vec::new(),
                    });
                }
                Some(_) => {
                    if !had_ws {
                        return Err(self.err("whitespace required before attribute"));
                    }
                    let an = self.name()?;
                    self.skip_ws();
                    self.expect('=')?;
                    self.skip_ws();
                    let v = self.attr_value()?;
                    if attrs.iter().any(|a| a.0 == an) {
                        return Err(self.err(&format!("duplicate attribute '{}'", an)));
                    }
                    attrs.push((an, v));
                }
                None => return Err(self.err("unexpected end inside start tag")),
            }
        }

        let mut children: Vec<Node> = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err(&format!("unclosed element <{}>", name))),
                Some('<') => {
                    if self.looking_at("</") {
                        if !text.is_empty() {
                            children.push(Node::Text(std::mem::take(&mut text)));
                        }
                        self.i += 2;
                        let closing = self.name()?;
                        self.skip_ws();
                        self.expect('>')?;
                        if closing != name {
                            return Err(self.err(&format!(
                                "mismatched closing tag </{}> for <{}>",
                                closing, name
                            )));
                        }
                        return Ok(Element {
                            name,
                            attrs,
                            children,
                        });
                    } else if self.looking_at("<!--") {
                        self.skip_comment()?;
                    } else if self.looking_at("<![CDATA[") {
                        self.i += 9;
                        let end = match self.find_from("]]>") {
                            Some(e) => e,
                            None => return Err(self.err("unterminated CDATA section")),
                        };
                        let t: String = self.s[self.i..end].iter().collect();
                        if t.chars().any(|c| !is_xml_char(c)) {
                            return Err(self.err("invalid character in CDATA section"));
                        }
                        text.push_str(&t);
                        self.i = end + 3;
                    } else if self.looking_at("<?") {
                        self.skip_pi()?;
                    } else if self.looking_at("<!") {
                        return Err(self.err("unsupported markup declaration"));
                    } else {
                        if !text.is_empty() {
                            children.push(Node::Text(std::mem::take(&mut text)));
                        }
                        let e = self.element(depth + 1)?;
                        children.push(Node::Element(e));
                    }
                }
                Some('&') => {
                    let c = self.entity()?;
                    text.push(c);
                }
                Some(c) => {
                    if !is_xml_char(c) {
                        return Err(self.err("invalid character in text"));
                    }
                    text.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let x = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- c -->\n<a x=\"1\"><b>t&amp;u</b><c/><![CDATA[<z>]]></a>\n";
        let r = parse(x).unwrap();
        assert_eq!(r.name, "a");
        assert_eq!(r.attr("x"), Some("1"));
        assert_eq!(r.child("b").unwrap().text(), "t&u");
        assert!(r.child("c").is_some());
        assert_eq!(r.text(), "t&u<z>");
        assert!(r.has_direct_text());
    }

    #[test]
    fn rejects() {
        assert!(parse("<a><b></a></b>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("<a/><b/>").is_err());
        assert!(parse("<!DOCTYPE a><a/>").is_err());
        assert!(parse("<a>&foo;</a>").is_err());
        assert!(parse("<a x=1/>").is_err());
        assert!(parse("<a x=\"1\" x=\"2\"/>").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn japanese_and_hyphen() {
        let r = parse("<law><e-gov-id>415AC</e-gov-id><name>道路交通法</name></law>").unwrap();
        assert_eq!(r.child("name").unwrap().text(), "道路交通法");
        assert!(!r.has_direct_text());
    }
}
