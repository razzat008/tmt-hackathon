use hackathon::translate::split_sentences;

#[test]
fn split_sentences_empty_or_whitespace() {
    assert!(split_sentences("").is_empty());
    assert!(split_sentences("   \n\t").is_empty());
}

#[test]
fn split_sentences_basic_punctuation() {
    let input = "Hello world. How are you? Fine!";
    let parts = split_sentences(input);
    assert_eq!(parts, vec!["Hello world.", "How are you?", "Fine!"]);
}

#[test]
fn split_sentences_devanagari_danda() {
    let input = "यो वाक्य। अर्को वाक्य";
    let parts = split_sentences(input);
    assert_eq!(parts, vec!["यो वाक्य।", "अर्को वाक्य"]);
}

#[test]
fn split_sentences_merges_short_fragments() {
    let input = "Hi. Ok? Great.";
    let parts = split_sentences(input);
    assert_eq!(parts, vec!["Hi. Ok?", "Great."]);
}
