//! Unicode script classification for the retrieval pipeline's language parity.
//!
//! Two distinct sets, and the distinction is load-bearing:
//! - **Bigram scripts** ([`is_bigram_script`]): scripts whose characters carry
//!   word-level meaning without a word delimiter, so the tokenizer emits
//!   overlapping character bigrams over them (Lucene's shipped `cjk_bigram`
//!   semantics) instead of one mega-token per punctuation-delimited run.
//! - **Unspaced scripts** ([`is_unspaced_script`]): the bigram scripts minus
//!   Hangul, because modern Korean orthography IS whitespace-delimited. Korean
//!   text chunks correctly on the word path and only its tokenization needs
//!   help, so it must not take the character-window chunking path.
//!
//! Pure, dependency-free code-point range checks over its inputs.

/// True when `c` belongs to a script the tokenizer bigrams: Han, Hiragana,
/// Katakana, Hangul, Thai, Lao, Khmer, or Myanmar. These scripts write words
/// without a delimiter (or, for Hangul, with sub-word syllable blocks), so
/// splitting them only on non-alphanumeric characters collapses whole clauses
/// into a single token that can never match a query term.
pub(crate) fn is_bigram_script(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // Han (CJK Unified Ideographs)
        | '\u{3400}'..='\u{4DBF}' // Han (CJK Unified Ideographs Extension A)
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{1100}'..='\u{11FF}' // Hangul Jamo
        | '\u{0E00}'..='\u{0E7F}' // Thai
        | '\u{0E80}'..='\u{0EFF}' // Lao
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{1000}'..='\u{109F}' // Myanmar
    )
}

/// True when `c` belongs to a script written without whitespace between words:
/// every [`is_bigram_script`] script except Hangul. Hangul is excluded because
/// modern Korean puts spaces between words, so Korean text still chunks
/// correctly on the whitespace/word path and must not be diverted to the
/// character-window path.
pub(crate) fn is_unspaced_script(c: char) -> bool {
    is_bigram_script(c) && !is_hangul(c)
}

/// True when `c` is a Hangul syllable or Jamo. Split out because Hangul is the
/// one script that is bigram-tokenized but not unspaced.
fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}')
}

/// Moves a bigram-script run into `out` as its overlapping character bigrams
/// (`ABCD` yields `AB`, `BC`, `CD`), then clears it. A single-character run
/// emits that character as a unigram, so a one-character word is not lost. A
/// no-op for an empty run.
///
/// Shared by the ranker's tokenizer (`crate::websearch::rank`) and the
/// citation audit's content-token extractor (`crate::websearch::cite_check`):
/// both accumulate an unspaced-script run character by character and need
/// identical bigram-emission behavior over it, so the logic lives here once
/// instead of twice.
pub(crate) fn push_bigram_tokens(out: &mut Vec<String>, run: &mut Vec<char>) {
    if run.is_empty() {
        return;
    }
    if run.len() == 1 {
        out.push(run[0].to_lowercase().collect());
    } else {
        for pair in run.windows(2) {
            out.push(pair.iter().collect::<String>().to_lowercase());
        }
    }
    run.clear();
}

/// The fraction of `text`'s non-whitespace characters that belong to an
/// unspaced script, in `0.0..=1.0`. Text with no non-whitespace character
/// scores `0.0`. Used to decide whether a page chunks on words or on a
/// character window.
pub(crate) fn unspaced_ratio(text: &str) -> f64 {
    let mut total = 0usize;
    let mut unspaced = 0usize;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        total += 1;
        if is_unspaced_script(c) {
            unspaced += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    unspaced as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigram_script_covers_every_listed_script() {
        for c in [
            '中', '\u{3400}', 'の', 'ア', '한', '\u{1100}', 'ก', 'ລ', 'ក', 'က',
        ] {
            assert!(is_bigram_script(c), "{c} should be a bigram script");
        }
    }

    #[test]
    fn bigram_script_excludes_latin_and_punctuation() {
        for c in ['a', 'Z', '9', 'ế', 'đ', ' ', '。', '，'] {
            assert!(!is_bigram_script(c), "{c} should not be a bigram script");
        }
    }

    #[test]
    fn unspaced_excludes_hangul_only() {
        assert!(is_unspaced_script('中'));
        assert!(is_unspaced_script('ก'));
        assert!(!is_unspaced_script('한'));
        assert!(!is_unspaced_script('\u{1100}'));
        assert!(!is_unspaced_script('a'));
    }

    #[test]
    fn unspaced_ratio_measures_non_whitespace_chars() {
        assert_eq!(unspaced_ratio(""), 0.0);
        assert_eq!(unspaced_ratio("   \n "), 0.0);
        assert_eq!(unspaced_ratio("plain english text"), 0.0);
        assert_eq!(unspaced_ratio("中文"), 1.0);
        // Whitespace is excluded from the denominator: 2 of 4 non-space chars.
        assert_eq!(unspaced_ratio("中文 ab"), 0.5);
    }

    #[test]
    fn unspaced_ratio_ignores_korean() {
        // Korean is bigram-tokenized but whitespace-delimited, so it must not
        // raise the unspaced ratio that diverts chunking to the char path.
        assert_eq!(unspaced_ratio("한국어 문장"), 0.0);
    }
}
