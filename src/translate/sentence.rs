pub fn split_sentences(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let parts = split_on_sentence_boundaries(text);

    let mut merged: Vec<String> = Vec::new();
    for part in parts {
        if part.chars().count() < 5 {
            if let Some(last) = merged.last_mut() {
                last.push(' ');
                last.push_str(&part);
            } else {
                merged.push(part);
            }
        } else {
            merged.push(part);
        }
    }

    merged
}

fn split_on_sentence_boundaries(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut last_split = 0;
    let mut prev_char: Option<char> = None;
    let mut iter = text.char_indices().peekable();

    while let Some((idx, ch)) = iter.next() {
        if ch.is_whitespace() {
            if let Some(prev) = prev_char {
                if is_sentence_delimiter(prev) {
                    let part = text[last_split..idx].trim();
                    if !part.is_empty() {
                        parts.push(part.to_string());
                    }

                    while let Some((_, next_ch)) = iter.peek() {
                        if next_ch.is_whitespace() {
                            iter.next();
                        } else {
                            break;
                        }
                    }

                    last_split = iter.peek().map(|(i, _)| *i).unwrap_or(text.len());
                    prev_char = None;
                    continue;
                }
            }
        }

        prev_char = Some(ch);
    }

    let remainder = text[last_split..].trim();
    if !remainder.is_empty() {
        parts.push(remainder.to_string());
    }

    parts
}

fn is_sentence_delimiter(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '।')
}
