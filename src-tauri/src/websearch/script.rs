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
//!
//! This module owns EVERY Unicode range the retrieval pipeline knows about: the
//! tokenizer/chunker sets above, and the per-script predicates
//! [`crate::websearch::lang`] resolves a query's language from. A second range
//! table anywhere else would drift from this one, so new ranges belong here.

/// True when `c` belongs to a script the tokenizer bigrams: Han, Hiragana,
/// Katakana, Hangul, Thai, Lao, Khmer, or Myanmar. These scripts write words
/// without a delimiter (or, for Hangul, with sub-word syllable blocks), so
/// splitting them only on non-alphanumeric characters collapses whole clauses
/// into a single token that can never match a query term.
pub(crate) fn is_bigram_script(c: char) -> bool {
    is_han(c)
        || is_kana(c)
        || is_hangul(c)
        || is_thai(c)
        || is_lao(c)
        || is_khmer(c)
        || is_myanmar(c)
}

/// True when `c` belongs to a script written without whitespace between words:
/// every [`is_bigram_script`] script except Hangul. Hangul is excluded because
/// modern Korean puts spaces between words, so Korean text still chunks
/// correctly on the whitespace/word path and must not be diverted to the
/// character-window path.
pub(crate) fn is_unspaced_script(c: char) -> bool {
    is_bigram_script(c) && !is_hangul(c)
}

/// True when `c` is a Han (CJK Unified Ideograph) character, including
/// Extension A. Han alone does not identify a language: Japanese writes Han
/// (kanji) alongside Kana, so a language check must test [`is_kana`] first.
pub(crate) fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// True when `c` is Hiragana or Katakana. Kana is the one script only Japanese
/// uses, so its presence is the decisive Japanese signal in Han-mixed text.
pub(crate) fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}')
}

/// True when `c` is a Hangul syllable or Jamo. Split out because Hangul is the
/// one script that is bigram-tokenized but not unspaced.
pub(crate) fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}')
}

/// True when `c` is Thai.
pub(crate) fn is_thai(c: char) -> bool {
    matches!(c, '\u{0E00}'..='\u{0E7F}')
}

/// True when `c` is Lao. Bigram-tokenized; no language code is derived from it
/// (Lao has no supported search channel), so only [`is_bigram_script`] reads it.
fn is_lao(c: char) -> bool {
    matches!(c, '\u{0E80}'..='\u{0EFF}')
}

/// True when `c` is Khmer. Bigram-tokenized only; see [`is_lao`].
fn is_khmer(c: char) -> bool {
    matches!(c, '\u{1780}'..='\u{17FF}')
}

/// True when `c` is Myanmar. Bigram-tokenized only; see [`is_lao`].
fn is_myanmar(c: char) -> bool {
    matches!(c, '\u{1000}'..='\u{109F}')
}

/// True when `c` is Arabic (base block plus the Arabic Supplement).
pub(crate) fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}')
}

/// True when `c` is Hebrew.
pub(crate) fn is_hebrew(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{05FF}')
}

/// True when `c` is Greek (base block plus Greek Extended, which carries the
/// polytonic accents).
pub(crate) fn is_greek(c: char) -> bool {
    matches!(c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
}

/// True when `c` is Cyrillic (base block plus the Cyrillic Supplement).
/// Detected but deliberately NOT mapped to a language: the script is shared by
/// Russian, Ukrainian, Bulgarian, Serbian and others, and guessing between them
/// from characters alone is wrong more often than it is right.
pub(crate) fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}' | '\u{0500}'..='\u{052F}')
}

/// True when `c` is a Vietnamese-DISTINCTIVE Latin character: the Latin
/// Extended Additional block (`U+1EA0`-`U+1EF9`, the tone-marked vowels such as
/// ạ ả ấ ầ ậ ế ệ ộ ợ ự), plus horned Ơ/ơ and Ư/ư and barred Đ/đ.
///
/// Deliberately narrow. Characters like à, é, ô and ê are excluded: French,
/// Portuguese, Spanish and Italian use them too, so counting them would flag
/// half of Europe as Vietnamese. Only characters no other major Latin
/// orthography uses count, which is what makes a share-of-tokens rule over them
/// meaningful (see [`crate::websearch::lang`]).
pub(crate) fn is_vietnamese_marker(c: char) -> bool {
    matches!(
        c,
        '\u{1EA0}'
            ..='\u{1EF9}'   // Latin Extended Additional (Vietnamese tones)
        | '\u{01A0}' | '\u{01A1}' // Ơ ơ
        | '\u{01AF}' | '\u{01B0}' // Ư ư
        | '\u{0110}' | '\u{0111}' // Đ đ
    )
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
    fn per_script_predicates_accept_their_script_and_reject_latin() {
        for (name, hit, predicate) in [
            ("han", '中', is_han as fn(char) -> bool),
            ("kana", 'の', is_kana),
            ("hangul", '한', is_hangul),
            ("thai", 'ก', is_thai),
            ("arabic", 'ا', is_arabic),
            ("hebrew", 'ש', is_hebrew),
            ("greek", 'Γ', is_greek),
            ("cyrillic", 'Ж', is_cyrillic),
            ("vietnamese", 'ệ', is_vietnamese_marker),
        ] {
            assert!(predicate(hit), "{name} should accept {hit}");
            assert!(!predicate('a'), "{name} should reject a Latin letter");
        }
    }

    #[test]
    fn vietnamese_marker_excludes_the_shared_european_diacritics() {
        // à ô ê é are French/Portuguese/Spanish too, so they carry no
        // Vietnamese signal; only the tone-marked and horned/barred letters do.
        for c in ['à', 'ô', 'ê', 'é', 'ă', 'â'] {
            assert!(!is_vietnamese_marker(c), "{c} is not distinctive");
        }
        for c in ['ạ', 'ế', 'ộ', 'ơ', 'ư', 'đ', 'Đ', 'Ơ', 'Ư'] {
            assert!(is_vietnamese_marker(c), "{c} is distinctive");
        }
    }

    #[test]
    fn unspaced_ratio_ignores_korean() {
        // Korean is bigram-tokenized but whitespace-delimited, so it must not
        // raise the unspaced ratio that diverts chunking to the char path.
        assert_eq!(unspaced_ratio("한국어 문장"), 0.0);
    }
}
