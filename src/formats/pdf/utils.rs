use once_cell::sync::Lazy;
use regex::Regex;

static HALANT_SPACE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\u{094D}\s+").expect("valid halant regex"));

pub fn fix_devanagari_clusters(text: &str) -> String {
    HALANT_SPACE_RE.replace_all(text, "\u{094D}").into_owned()
}

pub fn escape_pdf_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '(' => escaped.push_str("\\("),
            ')' => escaped.push_str("\\)"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
