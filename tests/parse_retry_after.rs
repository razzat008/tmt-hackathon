use std::time::Duration;

use hackathon::tmt::parse_retry_after;

#[test]
fn parse_retry_after_seconds() {
    let parsed = parse_retry_after(Some("5")).expect("expected duration");
    assert_eq!(parsed, Duration::from_secs(5));
}

#[test]
fn parse_retry_after_fractional_seconds() {
    let parsed = parse_retry_after(Some("2.5")).expect("expected duration");
    assert_eq!(parsed.as_millis(), 2500);
}

#[test]
fn parse_retry_after_zero_or_negative() {
    assert!(parse_retry_after(Some("0")).is_none());
    assert!(parse_retry_after(Some("-1")).is_none());
}

#[test]
fn parse_retry_after_missing_or_invalid() {
    assert!(parse_retry_after(None).is_none());
    assert!(parse_retry_after(Some("not-a-number")).is_none());
}
