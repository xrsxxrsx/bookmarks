//! Text normalisation, search matching and word counting.
//!
//! `normalize` is deliberately cross-cutting: three unrelated features depend on the
//! same folding rules, so they must not each invent their own:
//!   1. search matching (a query must hit `一，二，三` when typed as `一二三`)
//!   2. tag de-duplication (`Fluff` and `fluff` are one tag)
//!   3. filename heuristics, when comparing a parsed title against a filename
//!
//! Matching is plain substring search over the folded text rather than an FTS index.
//! For a personal library this is both faster and more accurate in Chinese: SQLite's
//! default tokenizer treats a run of Han characters as a single token (so "简单" would
//! never match "简单地杀个人"), and the trigram tokenizer cannot serve queries shorter
//! than three characters. Substring matching has neither limitation and needs no
//! segmenter. The threshold to revisit this is roughly 10k works.

use serde::{Deserialize, Serialize};

/// Folding rules:
///  * NFKC first, so full-width forms and compatibility characters collapse
///    (`，` -> `,`, `１` -> `1`, and full-width latin letters become ASCII).
///  * lowercase, so search is case-insensitive for latin scripts.
///  * every run of non-alphanumeric characters becomes a separator.
///
/// A separator becomes a space unless both neighbours are Han. Han text has no word
/// boundaries, so folding `一，二，三` to `一 二 三` would break substring matching: a
/// reader who types `一二三` (no comma) expects to find a title written with commas, and
/// removing the comma is the whole point of folding punctuation away. Keeping Han runs
/// contiguous achieves that. A space is still inserted where Han meets a space-separated
/// script (`粽驴abc` -> `粽驴 abc`) so the two never fuse into one bogus token.
///
/// `char::is_alphanumeric` is exactly the `Alphabetic | Numeric` Unicode property, so
/// Han, Kana and Hangul characters survive and are treated as word characters.
pub fn normalize(input: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let folded: String = input.nfkc().flat_map(char::to_lowercase).collect();

    let mut out = String::with_capacity(folded.len());
    let mut pending_separator = false;
    // `None` until the first character is emitted, so the first character never
    // triggers a script-change space.
    let mut prev_was_han: Option<bool> = None;
    for ch in folded.chars() {
        if !ch.is_alphanumeric() {
            pending_separator = true;
            continue;
        }

        let this_is_han = is_han(ch);
        // Space policy, decided from both neighbours:
        //   * Han continues into Han  -> never (a comma between Han just disappears, so
        //     a query typed without commas still matches the stored title), plus a
        //     separator is required when either side is not Han, since Kana, Hangul and
        //     latin all use word spacing and must not fuse across punctuation.
        // A script change with no separator always separates, which is what keeps
        // `粽驴abc` from becoming a single token.
        let need_space = match prev_was_han {
            None => false,
            Some(prev) => {
                let same_han_run = prev && this_is_han;
                if same_han_run {
                    false
                } else if pending_separator {
                    true
                } else {
                    prev || this_is_han
                }
            }
        };
        if need_space {
            out.push(' ');
        }

        out.push(ch);
        pending_separator = false;
        prev_was_han = Some(this_is_han);
    }
    out
}

/// A term of a folded query. Empty terms are dropped, so a query consisting only of
/// punctuation matches everything instead of nothing.
pub fn query_terms(query: &str) -> Vec<String> {
    normalize(query)
        .split(' ')
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// True when every term of `query` occurs somewhere in `haystack`.
///
/// All terms must match (AND), but they may match in different fields and in any
/// order, which is what makes `hannibal 翻译` useful for finding a translation pair.
/// Both sides must already be folded with [`normalize`].
pub fn matches(haystack: &str, query: &str) -> bool {
    query_terms(query)
        .iter()
        .all(|term| haystack.contains(term))
}

/// Relative relevance of `haystack` for `query`, used to order search results.
///
/// A whole-field match outranks a prefix match, which outranks a mid-string match,
/// so searching a title term floats that work to the top instead of leaving it
/// wherever the active sort put it.
pub fn relevance(haystack: &str, query: &str) -> u32 {
    let terms = query_terms(query);
    if terms.is_empty() {
        return 0;
    }
    let mut total = 0;
    for term in &terms {
        if haystack == term {
            total += 3;
        } else if haystack.starts_with(term) {
            total += 2;
        } else if haystack.contains(term) {
            total += 1;
        }
    }
    total
}

/// True for Han ideographs, including the common, extension-A and compatibility blocks.
pub fn is_han(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0xF900..=0xFAFF   // CJK Compatibility Ideographs
        | 0x20000..=0x2FA1F // Extensions B..F and compatibility supplement
    )
}

/// True for kana (Hiragana and Katakana), used by the language heuristic.
pub fn is_kana(ch: char) -> bool {
    // Hiragana and Katakana are adjacent blocks, so one range covers both.
    matches!(ch as u32, 0x3040..=0x30FF)
}

/// True for Hangul syllables and jamo.
pub fn is_hangul(ch: char) -> bool {
    matches!(ch as u32, 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F)
}

/// Counts words using the same shape as AO3's own counter: each Han character counts
/// as one word, and each run of other alphanumeric characters counts as one word.
///
/// This is an estimate, not a reproduction of AO3's number. Measured against AO3's
/// stated counts on real downloads the estimate runs high: +4.3% on a Chinese work
/// (23,159 vs 22,210) and +1.6% on an English one (68,925 vs 67,851). The offset is
/// small but real and not explained by punctuation stripping alone, so AO3's exact
/// rule is not simply "Han characters + latin words". AO3's own figure is therefore
/// always preferred when the file states it, and callers must record which of the two
/// they used so the UI can mark the difference.
pub fn count_words(text: &str) -> u64 {
    let mut count: u64 = 0;
    let mut in_token = false;
    for ch in text.chars() {
        if is_han(ch) {
            count += 1;
            in_token = false;
        } else if ch.is_alphanumeric() {
            if !in_token {
                count += 1;
                in_token = true;
            }
        } else {
            in_token = false;
        }
    }
    count
}

/// Characters that indicate the text is predominantly a CJK script, used to guess a
/// language when a file states none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRatios {
    pub total: u64,
    pub han: u64,
    pub kana: u64,
    pub hangul: u64,
}

impl ScriptRatios {
    pub fn of(text: &str) -> Self {
        let mut r = ScriptRatios::default();
        for ch in text.chars() {
            if ch.is_alphanumeric() {
                r.total += 1;
                if is_han(ch) {
                    r.han += 1;
                } else if is_kana(ch) {
                    r.kana += 1;
                } else if is_hangul(ch) {
                    r.hangul += 1;
                }
            }
        }
        r
    }

    /// Guesses an ISO 639-1 code from script composition, or `None` when the text
    /// gives no usable signal (e.g. pure ASCII, which is not exclusively English).
    ///
    /// Kana is checked before Han because Japanese prose mixes both, and any kana at
    /// all rules out Chinese.
    pub fn guess_language(&self) -> Option<&'static str> {
        if self.total == 0 {
            return None;
        }
        let pct = |n: u64| (n as f64) / (self.total as f64);
        if pct(self.kana) > 0.01 {
            return Some("ja");
        }
        if pct(self.hangul) > 0.05 {
            return Some("ko");
        }
        if pct(self.han) > 0.15 {
            return Some("zh");
        }
        None
    }
}

/// Strips tags and entities from an HTML fragment, returning readable plain text.
///
/// Used for non-AO3 HTML and for AO3 summaries, which arrive as markup
/// (`<p>`, `<br>`, `<em>`) but must be stored and displayed as text.
pub fn html_to_text(html: &str) -> String {
    let fragment = scraper::Html::parse_fragment(html);
    let mut out = String::new();
    collect_text(fragment.root_element(), &mut out);
    collapse_blank_lines(&out)
}

fn collect_text(node: scraper::ElementRef<'_>, out: &mut String) {
    use scraper::node::Node;

    for child in node.children() {
        match child.value() {
            Node::Text(text) => out.push_str(text),
            // Block-level elements end a line so paragraphs do not run together.
            Node::Element(el)
                if matches!(
                    el.name(),
                    "p" | "br" | "div" | "li" | "blockquote" | "h1" | "h2" | "h3"
                ) =>
            {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    out.push('\n');
                    collect_text(child_el, out);
                    out.push('\n');
                }
            }
            Node::Element(_) => {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    collect_text(child_el, out);
                }
            }
            _ => {}
        }
    }
}

/// Trims each line and collapses runs of blank lines, preserving paragraph breaks.
pub fn collapse_blank_lines(input: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blank_run = 0usize;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push(trimmed);
    }
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_punctuation_and_case() {
        assert_eq!(normalize("One, two, three"), "one two three");
        assert_eq!(normalize("ONE,TWO"), "one two");
        // Full-width punctuation must fold away, not survive as a character. Han runs
        // stay contiguous so the comma simply disappears, which is what makes a query
        // typed without commas still match.
        assert_eq!(normalize("一，二，三"), "一二三");
    }

    #[test]
    fn normalize_keeps_han_runs_contiguous_but_separates_scripts() {
        // Punctuation inside Han text vanishes, so substring search still works.
        assert_eq!(normalize("简单地杀个·人"), "简单地杀个人");
        // A Han/latin boundary keeps a space so the scripts do not fuse.
        assert_eq!(normalize("粽驴abc"), "粽驴 abc");
        assert_eq!(normalize("abc粽驴"), "abc 粽驴");
        // Latin words stay separated.
        assert_eq!(normalize("beta-test"), "beta test");
    }

    #[test]
    fn normalize_applies_nfkc_form() {
        // Full-width latin letters and digits collapse to ASCII.
        assert_eq!(normalize("ＡＢＣ１２３"), "abc123");
    }

    #[test]
    fn normalize_keeps_cjk_and_accents_intact() {
        // The bracketed prefix is pure Han, so no separators are introduced at all.
        assert_eq!(normalize("【粽驴】简单地杀个人"), "粽驴简单地杀个人");
        assert_eq!(normalize("Übersetzung"), "übersetzung");
    }

    #[test]
    fn normalize_collapses_repeated_separators() {
        assert_eq!(normalize("a --- b   c"), "a b c");
        assert_eq!(
            normalize("  leading and trailing  "),
            "leading and trailing"
        );
        assert_eq!(normalize("..."), "");
    }

    #[test]
    fn query_terms_drops_empty_pieces() {
        assert_eq!(query_terms("hannibal  翻译"), vec!["hannibal", "翻译"]);
        assert!(query_terms("!!!").is_empty());
    }

    #[test]
    fn matching_is_punctuation_and_case_insensitive() {
        // The real cases from the library: a CJK title written with full-width commas
        // and an English title with ASCII commas.
        let han = normalize("一，二，三");
        assert!(
            matches(&han, "一二三"),
            "CJK query without commas must match"
        );
        assert!(matches(&han, "一，二"), "CJK query with commas must match");

        let en = normalize("One, two, three");
        assert!(matches(&en, "one two three"));
        assert!(matches(&en, "ONE,TWO"));
    }

    #[test]
    fn matching_cjk_does_not_require_punctuation_to_be_typed() {
        // A longer Chinese title: the separators around a prefix must not block a hit.
        let hay = normalize("【粽驴】简单地杀个人");
        assert!(matches(&hay, "粽驴"));
        assert!(matches(&hay, "简单地杀"));
        assert!(
            matches(&hay, "【粽驴】"),
            "typing the brackets verbatim also works"
        );
        assert!(
            !matches(&hay, "杀个猪"),
            "a term absent from the text must not match"
        );
    }

    #[test]
    fn matching_requires_all_terms_but_not_contiguity() {
        let hay = normalize("One, two, three by Severus_divides_into_H");
        assert!(matches(&hay, "one three"), "terms may be non-adjacent");
        assert!(!matches(&hay, "one four"), "every term must be present");
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches(&normalize("anything"), ""));
        assert!(matches(&normalize("anything"), "!!!"));
    }

    #[test]
    fn relevance_orders_exact_over_prefix_over_substring() {
        let exact = relevance("hannibal", "hannibal");
        let prefix = relevance("hannibal lecter rising", "hannibal");
        let middle = relevance("young hannibal rising", "hannibal");
        assert!(exact > prefix, "{exact} should beat {prefix}");
        assert!(prefix > middle, "{prefix} should beat {middle}");
        assert_eq!(relevance("unrelated", "hannibal"), 0);
    }

    #[test]
    fn count_words_counts_han_per_character_and_latin_per_run() {
        assert_eq!(count_words("你好世界"), 4);
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("hello, world!"), 2);
        // Mixed text: 4 Han characters + 2 latin words + 1 number.
        assert_eq!(count_words("你好世界 hello world 42"), 7);
        // Punctuation alone is not a word.
        assert_eq!(count_words("!!! ... ---"), 0);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \n\t "), 0);
    }

    #[test]
    fn script_ratios_guess_language() {
        assert_eq!(
            ScriptRatios::of("这是一段完全中文的文本内容").guess_language(),
            Some("zh")
        );
        // Any kana at all means Japanese, even with heavy Han usage.
        assert_eq!(
            ScriptRatios::of("これは日本語のテキストです").guess_language(),
            Some("ja")
        );
        assert_eq!(
            ScriptRatios::of("안녕하세요 반갑습니다").guess_language(),
            Some("ko")
        );
        // Pure ASCII is not enough signal to call it English.
        assert_eq!(ScriptRatios::of("hello world").guess_language(), None);
        assert_eq!(ScriptRatios::of("").guess_language(), None);
    }

    #[test]
    fn html_to_text_strips_markup_and_keeps_paragraphs() {
        let html = "<p>First<br>line</p><p>Second <em>emphasis</em></p>";
        let text = html_to_text(html);
        assert!(text.contains("First"));
        assert!(text.contains("line"));
        assert!(text.contains("Second emphasis"));
        assert!(!text.contains('<'), "markup must not survive: {text}");
    }

    #[test]
    fn html_to_text_decodes_entities() {
        let text = html_to_text("<p>a &amp; b &lt;c&gt; &#39;d&#39;</p>");
        assert!(text.contains("a & b"), "got {text}");
        assert!(text.contains("<c>"), "got {text}");
    }

    #[test]
    fn blank_line_collapsing_preserves_single_breaks() {
        assert_eq!(collapse_blank_lines("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(collapse_blank_lines("\n\n  a  \n\n"), "a");
    }
}
