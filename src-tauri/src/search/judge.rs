//! Parse and normalize sufficiency-judge verdicts emitted by the LLM.
//!
//! Small local models frequently prepend polite chatter ("Sure, here is the
//! JSON:") or wrap output in ```json fences even when told not to. This
//! module is the tolerant boundary that extracts a single JSON object from
//! whatever the model actually returned, validates it into a
//! [`JudgeVerdict`], and enforces shape invariants:
//!
//! - `gap_queries` is capped at [`crate::config::defaults::DEFAULT_GAP_QUERIES_PER_ROUND`] and trimmed of blanks.
//! - When `sufficiency == Sufficient`, `gap_queries` must be empty (it would
//!   be incoherent otherwise; the pipeline never uses them and carrying stale
//!   ones pollutes persisted metadata).

use crate::search::types::{JudgeVerdict, Sufficiency};

/// Errors returned by [`parse_verdict`].
#[derive(Debug, thiserror::Error)]
pub enum JudgeParseError {
    /// The model response contained no `{...}` pair we could extract.
    #[error("no JSON object found in judge response")]
    NoJson,
    /// The extracted substring was syntactically JSON but did not match the
    /// expected verdict schema.
    #[error("invalid JSON: {0}")]
    BadJson(#[from] serde_json::Error),
}

/// Extract and deserialize a [`JudgeVerdict`] from raw LLM output.
///
/// Accepts chatter before or after the JSON object, markdown fences around
/// the object, or a clean object. The first balanced `{...}` pair is what
/// we parse.
pub fn parse_verdict(raw: &str) -> Result<JudgeVerdict, JudgeParseError> {
    let slice = extract_json_object(raw).ok_or(JudgeParseError::NoJson)?;
    Ok(serde_json::from_str(slice)?)
}

/// Exposed for the `llm` module which also deserializes a router+judge JSON
/// object from chatty model output.
pub fn extract_json_object_public(s: &str) -> Option<&str> {
    extract_json_object(s)
}

fn extract_json_object(s: &str) -> Option<&str> {
    let mut search_from = 0usize;
    while let Some(relative_start) = s[search_from..].find('{') {
        let start = search_from + relative_start;
        if let Some(end) = extract_json_object_end(&s[start..]) {
            return Some(&s[start..start + end]);
        }
        search_from = start + 1;
    }
    None
}

fn extract_json_object_end(candidate: &str) -> Option<usize> {
    let bytes = candidate.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }

    None
}

/// Apply shape invariants to a parsed verdict.
///
/// - Drops empty/whitespace-only `gap_queries`.
/// - Truncates to `max_gap_queries`.
/// - Clears `gap_queries` entirely when the verdict is `Sufficient`.
pub fn normalize_verdict(v: &mut JudgeVerdict, max_gap_queries: usize) {
    v.gap_queries.retain(|q| !q.trim().is_empty());
    v.gap_queries.truncate(max_gap_queries);
    if matches!(v.sufficiency, Sufficiency::Sufficient) {
        v.gap_queries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_clean_json() {
        let s = r#"{"sufficiency":"sufficient","reasoning":"yes","gap_queries":[]}"#;
        let v = parse_verdict(s).unwrap();
        assert_eq!(v.sufficiency, Sufficiency::Sufficient);
    }

    #[test]
    fn parse_strips_leading_chatty_preamble() {
        let s = "Sure, here is the JSON:\n```json\n{\"sufficiency\":\"partial\",\"reasoning\":\"x\",\"gap_queries\":[\"q1\"]}\n```";
        let v = parse_verdict(s).unwrap();
        assert_eq!(v.sufficiency, Sufficiency::Partial);
        assert_eq!(v.gap_queries, vec!["q1"]);
    }

    #[test]
    fn parse_rejects_unrecoverable_garbage() {
        assert!(matches!(
            parse_verdict("not json at all"),
            Err(JudgeParseError::NoJson)
        ));
    }

    #[test]
    fn parse_rejects_malformed_json_with_braces() {
        assert!(matches!(
            parse_verdict("{\"sufficiency\": not-a-string}"),
            Err(JudgeParseError::BadJson(_))
        ));
    }

    #[test]
    fn parse_accepts_missing_gap_queries_as_empty() {
        let s = r#"{"sufficiency":"insufficient","reasoning":"need more"}"#;
        let v = parse_verdict(s).unwrap();
        assert!(v.gap_queries.is_empty());
    }

    #[test]
    fn extract_handles_nested_braces() {
        let s = r#"noise {"sufficiency":"partial","reasoning":"nested {curly}","gap_queries":[]} trailing"#;
        let v = parse_verdict(s).unwrap();
        assert_eq!(v.sufficiency, Sufficiency::Partial);
    }

    #[test]
    fn normalize_drops_empty_gap_queries_and_caps_length() {
        let mut v = JudgeVerdict {
            sufficiency: Sufficiency::Partial,
            reasoning: "x".to_string(),
            gap_queries: vec![
                "".to_string(),
                "real".to_string(),
                "  ".to_string(),
                "q3".to_string(),
                "q4".to_string(),
                "q5".to_string(),
            ],
            parse_failure: false,
        };
        normalize_verdict(&mut v, 3);
        assert_eq!(
            v.gap_queries,
            vec!["real".to_string(), "q3".to_string(), "q4".to_string()]
        );
    }

    #[test]
    fn normalize_forces_empty_gap_queries_when_sufficient() {
        let mut v = JudgeVerdict {
            sufficiency: Sufficiency::Sufficient,
            reasoning: "x".to_string(),
            gap_queries: vec!["q1".to_string()],
            parse_failure: false,
        };
        normalize_verdict(&mut v, 3);
        assert!(v.gap_queries.is_empty());
    }

    #[test]
    fn extract_json_object_public_delegates() {
        assert_eq!(
            extract_json_object_public("prefix {\"a\":1} suffix"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json_object_public("no braces"), None);
    }

    #[test]
    fn parse_rejects_unbalanced_opening_brace() {
        // A '{' is found but the loop ends without a matching '}', so
        // extract_json_object returns None, which maps to NoJson.
        assert!(matches!(
            parse_verdict("{ unbalanced"),
            Err(JudgeParseError::NoJson)
        ));
    }

    #[test]
    fn parse_accepts_unmatched_open_brace_inside_reasoning_string() {
        let verdict = parse_verdict(
            r#"{"sufficiency":"partial","reasoning":"needs { more context","gap_queries":["q1"]}"#,
        )
        .expect("valid JSON with brace in string should parse");

        assert_eq!(verdict.sufficiency, Sufficiency::Partial);
        assert_eq!(verdict.reasoning, "needs { more context");
        assert_eq!(verdict.gap_queries, vec!["q1"]);
    }

    #[test]
    fn extract_json_object_ignores_closing_brace_inside_string() {
        let raw =
            r#"prefix {"sufficiency":"partial","reasoning":"saw } token","gap_queries":[]} suffix"#;

        assert_eq!(
            extract_json_object_public(raw),
            Some(r#"{"sufficiency":"partial","reasoning":"saw } token","gap_queries":[]}"#)
        );
    }

    #[test]
    fn parse_accepts_escaped_quotes_and_braces_inside_reasoning_string() {
        let verdict = parse_verdict(
            r#"{"sufficiency":"partial","reasoning":"saw \"quoted\" text with } and {","gap_queries":["q1"]}"#,
        )
        .expect("escaped quotes inside JSON strings should not break extraction");

        assert_eq!(verdict.reasoning, r#"saw "quoted" text with } and {"#);
    }

    #[test]
    fn parse_accepts_escaped_backslashes_inside_reasoning_string() {
        let verdict = parse_verdict(
            r#"{"sufficiency":"partial","reasoning":"path C:\\temp\\{cache}","gap_queries":[]}"#,
        )
        .expect("escaped backslashes inside JSON strings should not break extraction");

        assert_eq!(verdict.reasoning, r#"path C:\temp\{cache}"#);
    }

    #[test]
    fn parse_accepts_nested_object_members_while_finding_outer_boundary() {
        let verdict = parse_verdict(
            r#"noise {"sufficiency":"partial","reasoning":"x","gap_queries":[],"meta":{"source":"judge"}} trailing"#,
        )
        .expect("nested objects should not terminate extraction early");

        assert_eq!(verdict.sufficiency, Sufficiency::Partial);
        assert_eq!(verdict.reasoning, "x");
    }
}
