//! Query-language resolution and the per-channel request shapes it selects.
//!
//! Every outbound retrieval request used to be hardcoded English: DuckDuckGo's
//! `kl=us-en` and `Accept-Language: en-US`, Google News' `US:en` feed, the
//! `en.wikipedia.org` subdomain. A Vietnamese or Japanese question therefore
//! searched the English web and the writer had to answer from sources the user
//! could not read. This module resolves the language of the turn's query and
//! hands every channel the request shape that matches it.
//!
//! Resolution order, most certain first:
//! 1. **Script** ([`detect_script_lang`]): free, deterministic, and independent
//!    of how the machine is configured. Text written in Kana is Japanese no
//!    matter what `$LANG` says.
//! 2. **The classifier's `lang` field** (see [`crate::websearch::prepass`]): the
//!    model naming the language the USER wrote in. This is the layer that
//!    rescues shared-diacritic Vietnamese: `giá vàng hôm nay bao nhiêu` carries
//!    no Vietnamese-distinctive character at all, so the deterministic rule
//!    scores it zero and calls it English, yet it is plainly Vietnamese and is
//!    exactly the high-value local-price question class. No range table can see
//!    that; a model that understands the text can, and measured 12/12 on the
//!    non-English probe set including 8/8 English. It is UNTRUSTED INPUT all the
//!    same, and passes [`supported_lang`] before it can influence anything.
//! 3. **Locale** ([`locale_lang`]): the user's own `$LANG`, for the turns where
//!    neither the script nor the classifier named a language.
//! 4. **[`crate::config::defaults::SEARCH_LANG_DEFAULT`]**: English.
//!
//! The language is resolved ONCE per turn, from the user's ORIGINAL message, and
//! threaded down into every channel. It is never re-derived from a rewritten
//! query: the classifier rewrites into whatever wording retrieves best (measured:
//! it will happily emit an English companion query beside the native one), so a
//! rewritten query is a retrieval artifact, not a language signal.
//!
//! ## The allowlist is a security boundary, not a convenience
//!
//! A resolved language code reaches an outbound URL, and for Wikipedia it
//! reaches the HOSTNAME (`vi.wikipedia.org`). A runtime string interpolated into
//! a host is an injection primitive, so no runtime string ever gets there:
//! [`supported_lang`] is the ONLY way to obtain a language code from this
//! module, it is a `match` over compile-time literals, and it returns
//! `&'static str`. Every caller therefore holds a code that came out of the
//! allowlist, and every channel lookup below re-enters that same `match`.
//! Anything unrecognised, including a hostile `$LANG`, resolves to
//! `SEARCH_LANG_DEFAULT` rather than travelling any further.
//!
//! The allowlist is the 14 languages whose request shapes were verified live
//! against every channel, and whose Wikipedia editions clear a 100,000-article
//! bar (the smallest, Hindi, clears it by 170x).

use crate::config::defaults::{
    DDG_DEFAULT_REGION, MOJEEK_LANGUAGE_BIAS_BOOST, SEARCH_ACCEPT_LANGUAGE_FALLBACK,
    SEARCH_DEFAULT_ACCEPT_LANGUAGE, SEARCH_LANG_DEFAULT, SEARCH_LANG_SCRIPT_RATIO_MIN,
    SEARCH_LANG_VI_TOKEN_RATIO_MIN,
};
use crate::websearch::script::{
    is_arabic, is_cyrillic, is_greek, is_han, is_hangul, is_hebrew, is_kana, is_thai,
    is_vietnamese_marker,
};

/// One allowlisted language and the per-channel codes it selects. Every column
/// is a compile-time literal, so a value read off a row is `&'static str` by
/// construction and a runtime string can never impersonate one.
struct LangRow {
    /// ISO 639-1 code. Doubles as the Wikipedia subdomain, the Mojeek `lb`
    /// value, the Open-Meteo `language` value, and the Google News `hl`/`ceid`
    /// language token.
    code: &'static str,
    /// DuckDuckGo's `kl` REGION code. Irregular and not derivable from `code`:
    /// taken from DuckDuckGo's own region list.
    ddg_region: &'static str,
    /// The country whose Google News edition serves `code`. The feed's three
    /// parameters are all derived from this one pairing (see [`NewsLocale`]).
    news_region: &'static str,
}

/// The language allowlist, and the ONLY source of any language-derived value
/// that reaches a URL, a hostname, or a header.
///
/// Membership is not arbitrary: for each of these 14 languages, the DuckDuckGo
/// region, the Mojeek bias, the Google News triple, the Wikipedia subdomain and
/// the Open-Meteo language were verified live. A language missing from the table
/// has no verified shape on some channel, so serving it English is the honest
/// failure rather than a guess.
///
/// Two Vietnamese/Thai notes, because they look like typos and are not: `vi` is
/// `vn-en` (DuckDuckGo has no `vn-vi`; it does not exist) and `th` is `th-en`.
/// Neither region can carry a language, so their language bias rides entirely on
/// [`accept_language`].
const LANGS: &[LangRow] = &[
    LangRow {
        code: "en",
        ddg_region: "wt-wt",
        news_region: "US",
    },
    LangRow {
        code: "vi",
        ddg_region: "vn-en",
        news_region: "VN",
    },
    LangRow {
        code: "ja",
        ddg_region: "jp-jp",
        news_region: "JP",
    },
    LangRow {
        code: "zh",
        ddg_region: "cn-zh",
        news_region: "CN",
    },
    LangRow {
        code: "ko",
        ddg_region: "kr-kr",
        news_region: "KR",
    },
    LangRow {
        code: "th",
        ddg_region: "th-en",
        news_region: "TH",
    },
    LangRow {
        code: "ar",
        ddg_region: "xa-ar",
        news_region: "EG",
    },
    LangRow {
        code: "es",
        ddg_region: "es-es",
        news_region: "ES",
    },
    LangRow {
        code: "fr",
        ddg_region: "fr-fr",
        news_region: "FR",
    },
    LangRow {
        code: "de",
        ddg_region: "de-de",
        news_region: "DE",
    },
    LangRow {
        code: "pt",
        ddg_region: "br-pt",
        news_region: "BR",
    },
    LangRow {
        code: "ru",
        ddg_region: "ru-ru",
        news_region: "RU",
    },
    LangRow {
        code: "hi",
        ddg_region: "in-en",
        news_region: "IN",
    },
    LangRow {
        code: "id",
        ddg_region: "id-en",
        news_region: "ID",
    },
];

/// The allowlist row for `code`, or `None` when the code is not allowlisted.
/// The single gate: every channel lookup below goes through it, so an
/// unrecognised code has no path to any outbound value.
fn row(code: &str) -> Option<&'static LangRow> {
    LANGS.iter().find(|row| row.code == code)
}

/// The canonical `&'static str` for a supported ISO 639-1 code, or `None` for
/// anything else. `code` is matched exactly, so callers lowercase first (every
/// [`resolve_lang`] path does).
pub(crate) fn supported_lang(code: &str) -> Option<&'static str> {
    row(code).map(|row| row.code)
}

/// Every allowlisted ISO 639-1 code, in table order. The one place the classifier
/// grammar's `lang` enum comes from (see
/// [`crate::websearch::prepass::prepass_schema`]), so the set the model may emit
/// and the set that can reach a URL are the same set by construction rather than
/// by two lists agreeing.
pub(crate) fn supported_langs() -> Vec<&'static str> {
    LANGS.iter().map(|row| row.code).collect()
}

/// Resolves the language of the turn from the user's own `text`, the classifier's
/// `classifier_lang` (the language the model says the user wrote in, `""` when
/// there is none), and `user_locale`, falling back to [`SEARCH_LANG_DEFAULT`].
/// The returned code is always allowlisted (see [`supported_lang`]).
///
/// The pure core of this module: every input is a plain string and there is no
/// environment read, so the whole precedence chain is tested directly.
///
/// Precedence, and why:
/// - **Script first**: it is proof, not a hint. A user whose machine is English
///   can still ask in Japanese, and Kana says so with certainty. A script that
///   resolves to a language OUTSIDE the allowlist (Hebrew, Greek) does not stop
///   resolution: it falls through exactly as an undetectable script would, so the
///   later layers still get their say before English does.
/// - **Classifier second**: a judgement, not proof, so it never overrides a
///   certain script signal, but it sees what no range table can (Vietnamese
///   written with no distinctive diacritic). It is model output, so it is
///   filtered through [`supported_lang`] like any other untrusted string.
/// - **Locale third**: the machine's configuration is the weakest signal, and
///   letting it outrank the classifier is precisely the live regression this
///   ordering fixes: a `vi_VN` machine asking an English question must not be
///   sent to the Vietnamese web.
pub(crate) fn resolve_lang(text: &str, classifier_lang: &str, user_locale: &str) -> &'static str {
    detect_script_lang(text)
        .and_then(supported_lang)
        .or_else(|| supported_lang(&classifier_lang.trim().to_lowercase()))
        .or_else(|| locale_lang(user_locale))
        .unwrap_or(SEARCH_LANG_DEFAULT)
}

/// Reads the language subtag of a BCP-47-ish locale string, or `None` when it
/// names no supported language.
///
/// Parses defensively because both separators genuinely reach us:
/// `commands.rs::user_locale` returns the `$LANG` prefix, which macOS writes with
/// an UNDERSCORE (`en_US`), while its own hardcoded fallback uses a HYPHEN
/// (`en-US`). Splitting on either, taking the first subtag and lowercasing it
/// absorbs both forms (and `vi_VN.UTF-8`-style leftovers) without any caller
/// having to normalise first.
pub(crate) fn locale_lang(user_locale: &str) -> Option<&'static str> {
    let subtag = user_locale
        .split(['-', '_', '.'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    supported_lang(&subtag)
}

/// Names the language of `text` from its script alone, or `None` when the script
/// cannot name one.
///
/// Every script test is a SHARE of the alphabetic characters, never a presence
/// check, and must clear [`SEARCH_LANG_SCRIPT_RATIO_MIN`]: one quoted foreign
/// character inside an otherwise English question must not redirect the whole
/// turn's search. Order matters:
/// - Kana is tested before Han, because Japanese mixes Han (kanji) with Kana
///   while Chinese never uses Kana. The Japanese share counts Han AND Kana
///   together, since kanji-heavy Japanese carries little Kana.
/// - Han with neither Kana nor Hangul is Chinese.
/// - Cyrillic is deliberately absent: it is shared across Russian, Ukrainian,
///   Bulgarian and Serbian, and we do not guess. It returns `None` and lets the
///   locale decide.
/// - Latin returns `None` too, EXCEPT Vietnamese (see [`is_vietnamese`]), whose
///   diacritics are distinctive enough to name it.
pub(crate) fn detect_script_lang(text: &str) -> Option<&'static str> {
    let alphabetic = text.chars().filter(|c| c.is_alphabetic()).count();
    if alphabetic == 0 {
        return None;
    }
    let share = |count: usize| count as f64 / alphabetic as f64;
    let count_of = |predicate: fn(char) -> bool| text.chars().filter(|c| predicate(*c)).count();

    let kana = count_of(is_kana);
    let han = count_of(is_han);
    if kana > 0 && share(kana + han) >= SEARCH_LANG_SCRIPT_RATIO_MIN {
        return Some("ja");
    }
    if share(count_of(is_hangul)) >= SEARCH_LANG_SCRIPT_RATIO_MIN {
        return Some("ko");
    }
    if share(han) >= SEARCH_LANG_SCRIPT_RATIO_MIN {
        return Some("zh");
    }
    for (predicate, lang) in [
        (is_thai as fn(char) -> bool, "th"),
        (is_arabic, "ar"),
        (is_hebrew, "he"),
        (is_greek, "el"),
    ] {
        if share(count_of(predicate)) >= SEARCH_LANG_SCRIPT_RATIO_MIN {
            return Some(lang);
        }
    }
    // Cyrillic is recognised only to say, explicitly, that it names nothing.
    if share(count_of(is_cyrillic)) >= SEARCH_LANG_SCRIPT_RATIO_MIN {
        return None;
    }
    is_vietnamese(text).then_some("vi")
}

/// Whether `text` is Vietnamese, by the share of its whitespace tokens carrying
/// at least one Vietnamese-distinctive character
/// ([`crate::websearch::script::is_vietnamese_marker`]).
///
/// A token share, not a character share and emphatically not a presence check.
/// Vietnamese diacritics arrive in English on loanwords, and a loanword occupies
/// exactly one token: "what does phở mean" scores `0.25` and stays English,
/// while Vietnamese sentences spread marks across their tokens ("thời tiết Hà
/// Nội hôm nay" scores `0.50`). The bar is
/// [`SEARCH_LANG_VI_TOKEN_RATIO_MIN`].
///
/// The rule is deliberately conservative: Vietnamese written with few marked
/// tokens scores below the bar and falls through to the user's locale, which
/// names Vietnamese correctly for the users who actually write it. A false
/// negative costs a locale lookup; a false positive would search the Vietnamese
/// web for an English question.
///
/// Called only from [`detect_script_lang`], and only after it has established
/// that `text` holds at least one alphabetic character, so it always has at
/// least one token and the division below cannot be by zero.
fn is_vietnamese(text: &str) -> bool {
    let tokens = text.split_whitespace().count();
    let marked = text
        .split_whitespace()
        .filter(|token| token.chars().any(is_vietnamese_marker))
        .count();
    marked as f64 / tokens as f64 >= SEARCH_LANG_VI_TOKEN_RATIO_MIN
}

/// The resolved language when it is NOT the default, so a caller can OMIT a
/// language parameter entirely on an English query.
///
/// Every channel here already defaults to English, so sending `lb=en` or
/// `language=en` would only restate the endpoint's own default while changing a
/// request shape that is verified good. `None` means "send nothing".
fn non_default_lang(lang: &str) -> Option<&'static str> {
    supported_lang(lang).filter(|resolved| *resolved != SEARCH_LANG_DEFAULT)
}

/// DuckDuckGo's `kl` value for `lang`: a REGION code, not a language one.
///
/// The codes are irregular and are taken from DuckDuckGo's own region list, so
/// they cannot be derived from the language code (`ja` is `jp-jp`, `ko` is
/// `kr-kr`, `ar` is `xa-ar`). Two languages have no vernacular region at all and
/// take an English-medium regional code: Vietnamese is `vn-en` (there is no
/// `vn-vi`) and Thai is `th-en`. For those two especially, the language bias has
/// to come from [`accept_language`]; the region code alone cannot deliver it.
///
/// An unresolved language gets [`DDG_DEFAULT_REGION`] (`wt-wt`, worldwide),
/// which is also English's own row: the `us-en` that used to be forced onto
/// every query is gone.
pub(crate) fn ddg_region(lang: &str) -> &'static str {
    row(lang).map_or(DDG_DEFAULT_REGION, |row| row.ddg_region)
}

/// The `Accept-Language` header for `lang`.
///
/// This is the engine tier's only language lever: DuckDuckGo's HTML endpoint has
/// no language selector, and its `kl` sets the region. A hardcoded English
/// header therefore FIGHTS a non-English region and keeps the results English,
/// which is exactly what used to happen. A non-English language asks for itself
/// first and keeps English as a weighted fallback
/// ([`SEARCH_ACCEPT_LANGUAGE_FALLBACK`]) so a page with no local edition still
/// ranks over an arbitrary third language.
pub(crate) fn accept_language(lang: &str) -> String {
    match non_default_lang(lang) {
        Some(resolved) => format!("{resolved}{SEARCH_ACCEPT_LANGUAGE_FALLBACK}"),
        None => SEARCH_DEFAULT_ACCEPT_LANGUAGE.to_string(),
    }
}

/// Mojeek's documented language-bias pair for `lang`: `lb` (the ISO 639-1 code)
/// and `lbb` (the bias strength, at its [`MOJEEK_LANGUAGE_BIAS_BOOST`] maximum).
/// `None` for English, which is Mojeek's own default and needs no parameters.
pub(crate) fn mojeek_language_bias(lang: &str) -> Option<(&'static str, &'static str)> {
    non_default_lang(lang).map(|resolved| (resolved, MOJEEK_LANGUAGE_BIAS_BOOST))
}

/// Open-Meteo's geocoding `language` value for `lang`, which localises the place
/// names it returns. Always sends a value, because this request always did:
/// an unallowlisted language sends [`SEARCH_LANG_DEFAULT`], leaving the English
/// request exactly as it was.
pub(crate) fn geocode_language(lang: &str) -> &'static str {
    supported_lang(lang).unwrap_or(SEARCH_LANG_DEFAULT)
}

/// The Wikipedia subdomain for `lang`, e.g. `vi` for `vi.wikipedia.org`.
///
/// Every allowlisted language IS its own subdomain, so this is the identity map
/// over [`supported_lang`], with the point being that it is `&'static str` all
/// the way: an unallowlisted code cannot become a hostname, it becomes `en`.
pub(crate) fn wiki_subdomain(lang: &str) -> &'static str {
    supported_lang(lang).unwrap_or(SEARCH_LANG_DEFAULT)
}

/// Google News' locale for one language: the language code and the ONE region
/// paired with it.
///
/// The feed takes three parameters (`hl`, `gl`, `ceid`) that must agree, and
/// disagreement fails SILENTLY: a mismatched or garbage triple returns HTTP 200
/// with a full feed, in ENGLISH (verified live). Mislabelled English news is the
/// worst outcome available, so the triple is never assembled from three inputs.
/// It is derived from this single row, and cannot disagree by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NewsLocale {
    /// The ISO 639-1 language, allowlisted.
    lang: &'static str,
    /// The country whose edition of the feed serves that language.
    region: &'static str,
}

impl NewsLocale {
    /// The `hl` (interface language) value, `<lang>-<REGION>`.
    pub(crate) fn hl(&self) -> String {
        format!("{}-{}", self.lang, self.region)
    }

    /// The `gl` (geography) value: the region alone.
    pub(crate) fn gl(&self) -> &'static str {
        self.region
    }

    /// The `ceid` (country edition) value, `<REGION>:<lang>`. Dominates `hl` and
    /// `gl` when they conflict, which is precisely why all three derive from the
    /// same row.
    pub(crate) fn ceid(&self) -> String {
        format!("{}:{}", self.region, self.lang)
    }
}

/// Google News' locale row for `lang`, or `None` when the language has none.
///
/// Every row below was verified live: the feed returned HTTP 200 with 50+ items
/// whose headlines were in the requested language. A language with no row sends
/// no locale parameters at all, which serves the English feed. That is
/// deliberate: an unverified triple would serve the English feed anyway, but
/// labelled as if it were the user's language.
pub(crate) fn news_locale(lang: &str) -> Option<NewsLocale> {
    row(lang).map(|row| NewsLocale {
        lang: row.code,
        region: row.news_region,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every allowlisted language, so a channel table can be swept exhaustively.
    const ALL: &[&str] = &[
        "en", "vi", "ja", "zh", "ko", "th", "ar", "es", "fr", "de", "pt", "ru", "hi", "id",
    ];

    // ── script detection ─────────────────────────────────────────────────────

    #[test]
    fn script_detection_names_the_scripts_it_can() {
        // Japanese: Han + Kana mixed. Kana is checked first, so this is `ja`,
        // never `zh`.
        assert_eq!(detect_script_lang("東京の天気は"), Some("ja"));
        assert_eq!(detect_script_lang("ニュース"), Some("ja"));
        assert_eq!(detect_script_lang("서울 날씨 어때"), Some("ko"));
        assert_eq!(detect_script_lang("北京今天天气"), Some("zh"));
        assert_eq!(detect_script_lang("สภาพอากาศวันนี้"), Some("th"));
        assert_eq!(detect_script_lang("ما هو الطقس اليوم"), Some("ar"));
        assert_eq!(detect_script_lang("מה מזג האוויר"), Some("he"));
        assert_eq!(detect_script_lang("τι καιρό κάνει"), Some("el"));
    }

    #[test]
    fn script_detection_declines_cyrillic_latin_and_empty() {
        // Cyrillic is shared across ru/uk/bg/sr: we do not guess.
        assert_eq!(detect_script_lang("какая сегодня погода"), None);
        assert_eq!(detect_script_lang("what is the weather today"), None);
        assert_eq!(detect_script_lang("quel temps fait-il"), None);
        assert_eq!(detect_script_lang(""), None);
        assert_eq!(detect_script_lang("123 !!!"), None);
    }

    #[test]
    fn one_stray_character_never_flips_the_language() {
        // An English question quoting a single Han character scores 1/13 and
        // stays English. This is the whole point of the ratio.
        assert_eq!(detect_script_lang("what does 中 mean"), None);
        assert_eq!(detect_script_lang("what does の mean"), None);
        // A genuinely mixed query still resolves: 4 Kana of 10 alphabetic.
        assert_eq!(detect_script_lang("iPhone レビュー"), Some("ja"));
    }

    // ── Vietnamese ───────────────────────────────────────────────────────────

    #[test]
    fn vietnamese_resolves_from_marked_token_share() {
        // thời/tiết/Nội are marked, Hà/hôm/nay are not: 3 of 6 = 0.50.
        assert_eq!(detect_script_lang("thời tiết Hà Nội hôm nay"), Some("vi"));
        // Việt/tỉnh marked: 2 of 6 = 0.33.
        assert_eq!(detect_script_lang("Việt Nam có bao nhiêu tỉnh"), Some("vi"));
        // đội/tuyển/Việt/đá marked: 4 of 7 = 0.57.
        assert_eq!(
            detect_script_lang("đội tuyển Việt Nam đá với ai"),
            Some("vi")
        );
    }

    #[test]
    fn english_question_with_a_vietnamese_loanword_stays_english() {
        // THE counterexample: one marked token of four = 0.25, under the bar.
        // A presence check would call this Vietnamese and search the Vietnamese
        // web for an English question.
        assert_eq!(detect_script_lang("what does phở mean"), None);
        // And with the classifier agreeing it is English (measured: it does),
        // the loanword trap stays shut all the way through resolution.
        assert_eq!(resolve_lang("what does phở mean", "en", "en_US"), "en");
        // Longer English text quoting several Vietnamese dishes stays English
        // too, because the marked share only falls as the text grows.
        assert_eq!(
            detect_script_lang("i want to cook phở and bún chả at home this weekend"),
            None
        );
    }

    // ── the classifier's `lang` field ────────────────────────────────────────

    #[test]
    fn classifier_lang_rescues_shared_diacritic_vietnamese() {
        // THE case the field exists for: not one character of this question is
        // Vietnamese-distinctive, so the deterministic rule scores it 0.000 and
        // names nothing. It is Vietnamese, and it is the highest-value question
        // class there is (a local price, in the local currency).
        for question in [
            "giá vàng hôm nay bao nhiêu",
            "giá xăng hôm nay bao nhiêu",
            "hôm nay có tin gì mới không",
        ] {
            assert_eq!(detect_script_lang(question), None);
            assert_eq!(resolve_lang(question, "vi", "en_US"), "vi");
        }
    }

    #[test]
    fn script_beats_the_classifier_and_the_classifier_beats_the_locale() {
        // Script is certain, so it outranks the model's judgement even when the
        // model disagrees.
        assert_eq!(resolve_lang("東京の天気は", "en", "en_US"), "ja");
        // No script signal: the classifier decides, over the locale.
        assert_eq!(resolve_lang("giá vàng hôm nay", "vi", "en_US"), "vi");
        // And the same precedence protects the reverse case, which is the live
        // regression: an English question on a Vietnamese machine.
        assert_eq!(resolve_lang("what is the gold price", "en", "vi_VN"), "en");
        // No classifier signal: the locale decides, over the default.
        assert_eq!(resolve_lang("what is the gold price", "", "vi_VN"), "vi");
        // Nothing at all: English.
        assert_eq!(resolve_lang("what is the gold price", "", ""), "en");
    }

    #[test]
    fn an_unvalidated_classifier_lang_never_reaches_a_url() {
        // The grammar enum-constrains `lang`, but it is still model output, so
        // it is treated as hostile: anything outside the allowlist is discarded
        // and resolution simply continues to the next layer.
        for hostile in ["evil", "../../en", "xx", "", "  ", "vi.evil.com", "he"] {
            // No locale either: nothing survives, so English.
            let resolved = resolve_lang("plain english question", hostile, "");
            assert_eq!(
                resolved, "en",
                "classifier lang {hostile:?} escaped validation"
            );
            // A locale still gets its say after the hostile value is dropped, so
            // a bad `lang` degrades to the layer below rather than to English.
            assert_eq!(
                resolve_lang("plain english question", hostile, "ja_JP"),
                "ja"
            );
        }
    }

    #[test]
    fn the_grammar_enum_is_exactly_the_allowlist() {
        // One table, so the set the model may emit and the set that can reach a
        // hostname cannot drift apart.
        assert_eq!(supported_langs(), ALL.to_vec());
    }

    // ── locale fallback ──────────────────────────────────────────────────────

    #[test]
    fn locale_parses_both_separators_and_a_codeset_suffix() {
        assert_eq!(locale_lang("vi_VN"), Some("vi"));
        assert_eq!(locale_lang("vi-VN"), Some("vi"));
        assert_eq!(locale_lang("vi_VN.UTF-8"), Some("vi"));
        assert_eq!(locale_lang("JA"), Some("ja"));
        assert_eq!(locale_lang("en_US"), Some("en"));
        assert_eq!(locale_lang("en-US"), Some("en"));
    }

    #[test]
    fn unknown_or_hostile_locale_falls_back_to_the_default() {
        assert_eq!(locale_lang("xx_YY"), None);
        assert_eq!(locale_lang(""), None);
        assert_eq!(locale_lang("../../etc/passwd"), None);
        assert_eq!(resolve_lang("plain english", "", "xx_YY"), "en");
        assert_eq!(resolve_lang("plain english", "", "evil.example.com"), "en");
    }

    #[test]
    fn script_beats_locale_and_locale_beats_the_default() {
        // Script wins: a Japanese question from an English machine is Japanese.
        assert_eq!(resolve_lang("東京の天気は", "", "en_US"), "ja");
        // No script signal: the locale decides.
        assert_eq!(resolve_lang("weather in Hanoi", "", "vi_VN"), "vi");
        assert_eq!(resolve_lang("погода сегодня", "", "ru_RU"), "ru");
        // Neither: English.
        assert_eq!(resolve_lang("weather in Hanoi", "", ""), "en");
        // A detected script outside the allowlist de-escalates to the locale,
        // never to a request shape we never verified.
        assert_eq!(resolve_lang("מה מזג האוויר", "", "fr_FR"), "fr");
        assert_eq!(resolve_lang("τι καιρό κάνει", "", "en_US"), "en");
    }

    // ── allowlist ────────────────────────────────────────────────────────────

    #[test]
    fn every_resolution_lands_inside_the_allowlist() {
        for text in [
            "東京の天気は",
            "thời tiết Hà Nội",
            "מה מזג האוויר",
            "what is the weather",
        ] {
            for locale in ["en_US", "he_IL", "xx_YY", "'; DROP TABLE--", ""] {
                for classifier in ["", "vi", "he", "../../en", "'; DROP TABLE--"] {
                    let lang = resolve_lang(text, classifier, locale);
                    assert!(
                        ALL.contains(&lang),
                        "resolve_lang({text:?}, {classifier:?}, {locale:?}) escaped the allowlist: {lang}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unallowlisted_code_cannot_reach_a_url_or_a_host() {
        for code in ["he", "el", "xx", "", "evil.com", "../../en"] {
            assert_eq!(supported_lang(code), None);
            // Wikipedia's HOSTNAME: the strictest boundary. Never the input.
            assert_eq!(wiki_subdomain(code), "en");
            // DuckDuckGo's region, Mojeek's bias, Open-Meteo's language,
            // Google News' triple: none of them can carry the code either.
            assert_eq!(ddg_region(code), DDG_DEFAULT_REGION);
            assert_eq!(mojeek_language_bias(code), None);
            assert_eq!(geocode_language(code), "en");
            assert_eq!(news_locale(code), None);
            assert_eq!(accept_language(code), SEARCH_DEFAULT_ACCEPT_LANGUAGE);
        }
    }

    // ── channel shapes ───────────────────────────────────────────────────────

    #[test]
    fn ddg_region_is_the_verified_irregular_code() {
        // Vietnamese has NO vernacular region code: `vn-vi` does not exist.
        assert_eq!(ddg_region("vi"), "vn-en");
        assert_eq!(ddg_region("th"), "th-en");
        assert_eq!(ddg_region("ja"), "jp-jp");
        assert_eq!(ddg_region("ko"), "kr-kr");
        assert_eq!(ddg_region("ar"), "xa-ar");
        assert_eq!(ddg_region("hi"), "in-en");
        assert_eq!(ddg_region("id"), "id-en");
        assert_eq!(ddg_region("fr"), "fr-fr");
        // English and unresolved both go worldwide, not United States.
        assert_eq!(ddg_region("en"), "wt-wt");
    }

    #[test]
    fn accept_language_follows_the_resolved_language() {
        // Vietnamese: `vn-en` cannot deliver Vietnamese-language results, so
        // this header is the only thing that can.
        assert_eq!(accept_language("vi"), "vi,en;q=0.5");
        assert_eq!(accept_language("ja"), "ja,en;q=0.5");
        // English is unchanged from what every request sent before.
        assert_eq!(accept_language("en"), "en-US,en;q=0.9");
    }

    #[test]
    fn mojeek_omits_the_default_language_and_geocode_always_states_it() {
        // Mojeek: English is its own default, so an English query keeps the
        // parameterless URL it always sent.
        assert_eq!(mojeek_language_bias("vi"), Some(("vi", "100")));
        assert_eq!(mojeek_language_bias("en"), None);
        // Open-Meteo: this request always carried `language=en`, so it still
        // does, and a resolved language simply replaces the value.
        assert_eq!(geocode_language("ja"), "ja");
        assert_eq!(geocode_language("en"), "en");
    }

    #[test]
    fn wiki_subdomain_is_the_language_itself() {
        for lang in ALL {
            assert_eq!(wiki_subdomain(lang), *lang);
        }
    }

    #[test]
    fn every_news_row_is_internally_consistent() {
        // The feed fails silently on a mismatched triple, so consistency is
        // asserted structurally over every row, not spot-checked.
        for lang in ALL {
            let locale = news_locale(lang).expect("every allowlisted language has a news row");
            assert_eq!(locale.hl(), format!("{}-{}", lang, locale.gl()));
            assert_eq!(locale.ceid(), format!("{}:{}", locale.gl(), lang));
            assert!(
                locale.gl().chars().all(|c| c.is_ascii_uppercase()),
                "{lang}: region must be an uppercase country code"
            );
        }
    }

    #[test]
    fn news_rows_are_the_live_verified_pairings() {
        assert_eq!(news_locale("en").unwrap().hl(), "en-US");
        assert_eq!(news_locale("en").unwrap().ceid(), "US:en");
        assert_eq!(news_locale("vi").unwrap().hl(), "vi-VN");
        assert_eq!(news_locale("zh").unwrap().ceid(), "CN:zh");
        assert_eq!(news_locale("pt").unwrap().ceid(), "BR:pt");
        assert_eq!(news_locale("ar").unwrap().gl(), "EG");
    }
}
