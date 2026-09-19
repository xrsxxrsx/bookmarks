//! Parser for AO3's "download work" HTML.
//!
//! The downloaded file carries no `<meta name="ao3:*">` metadata — that was a wrong
//! assumption worth recording, because a parser written against non-existent tags fails
//! silently rather than loudly. Everything comes from the document structure instead:
//!
//! | field            | source                                                      |
//! |------------------|-------------------------------------------------------------|
//! | work id / url    | `p.message > a[href*=/works/]`                               |
//! | title            | `div.meta > h1`  (NOT `<title>`, see `parse_title`)           |
//! | author           | `div.byline a[rel=author]`                                    |
//! | tag list         | `dl.tags` as `dt` -> following `dd` siblings (NEVER by index)  |
//! | language         | `dt "Language:"` -> next `dd`                                 |
//! | stats            | `dt "Stats:"` -> next `dd`, scanned as text                   |
//! | summary          | `p "Summary"` + next `blockquote.userstuff`                   |
//! | related works    | the `ul` after `dl.tags` inside `#preface`                    |
//!
//! Two traps this module is built to avoid:
//!  1. `<title>` is `Title - Author - Fandom` joined by " - ". Any title or author
//!     containing " - " breaks splitting, so the DOM is used instead.
//!  2. `dl.tags` omits absent rows. Reading values positionally shifts every later
//!     field when, say, a Gen work has no Relationship — so rows are matched by the
//!     `dt` label text.
//!
//! A related-works label must be matched *within that `ul`*: prose elsewhere in the
//! file can contain phrases like "inspired by", and matching them would fabricate
//! relations.

use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};

use crate::date::DateValue;
use crate::text;

/// Where an extracted value came from, so the UI can mark the shaky ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Ao3,
    Csv,
    Epub,
    Filename,
    Manual,
    Estimated,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Ao3 => "ao3",
            Source::Csv => "csv",
            Source::Epub => "epub",
            Source::Filename => "filename",
            Source::Manual => "manual",
            Source::Estimated => "estimated",
        }
    }
}

/// How two works are related. Kept distinct because only a translation should be
/// paired automatically; "inspired by" and remixes are a different claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// This work is a translation of the linked work ("A translation of ...").
    TranslationOf,
    /// The linked work is a translation into `language` ("Translation into X available:").
    TranslationInto,
    RemixOf,
    RelatedTo,
}

impl RelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationKind::TranslationOf | RelationKind::TranslationInto => "translation",
            RelationKind::RemixOf => "remix",
            RelationKind::RelatedTo => "related",
        }
    }

    pub fn is_translation(self) -> bool {
        matches!(
            self,
            RelationKind::TranslationOf | RelationKind::TranslationInto
        )
    }
}

/// A work referenced by the file but not necessarily present in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedWork {
    pub kind: RelationKind,
    /// AO3 work id of the referenced work, when the link exposes one.
    pub work_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    /// ISO 639-1 code for translations. AO3 marks this with `lang="zh"` on the
    /// language span, so translations need no language guessing at all.
    pub language: Option<String>,
    /// The language as displayed, e.g. `中文-普通话 國語`.
    pub language_label: Option<String>,
}

/// AO3's own tag groupings.
///
/// `Default` is derived: every group starts empty, which is the correct state for a work
/// that has no such tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tags {
    pub rating: Vec<String>,
    pub warnings: Vec<String>,
    pub category: Vec<String>,
    pub fandoms: Vec<String>,
    pub relationships: Vec<String>,
    pub characters: Vec<String>,
    /// AO3's own freeform tags. Stored apart from the user's tags, see the plan:
    /// mixing them would dilute the user's own filtering vocabulary.
    pub additional: Vec<String>,
}

impl Tags {
    /// Every AO3 tag as one list, for the "import AO3 tags as my tags" action.
    pub fn all(&self) -> Vec<String> {
        let mut out = Vec::new();
        for group in [
            &self.rating,
            &self.warnings,
            &self.category,
            &self.fandoms,
            &self.relationships,
            &self.characters,
            &self.additional,
        ] {
            out.extend(group.iter().cloned());
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub published: Option<DateValue>,
    pub completed: Option<DateValue>,
    /// AO3's own word count. Preferred over any estimate because it is exact.
    pub words: Option<u64>,
    pub chapters: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ao3Work {
    pub work_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub language_label: Option<String>,
    /// Mapped ISO 639-1 code. AO3 only states a display label such as
    /// `中文-普通话 國語`, so a lookup table is required.
    pub language: Option<String>,
    pub tags: Tags,
    pub stats: Stats,
    pub collections: Vec<String>,
    pub related: Vec<RelatedWork>,
}

impl Ao3Work {
    /// Whether this really is an AO3 download.
    ///
    /// Both markers must be present: the chapter container plus a `/works/<id>` link in
    /// the preface message. A generic HTML file that merely mentions AO3 must not be
    /// parsed as one, or it would import as an empty record.
    pub fn is_confident(&self) -> bool {
        self.work_id.is_some() && self.title.is_some()
    }

    /// The relation this work has to another, derived from its own related list.
    pub fn role(&self) -> Option<&'static str> {
        if self
            .related
            .iter()
            .any(|r| r.kind == RelationKind::TranslationOf)
        {
            Some("translation")
        } else if self
            .related
            .iter()
            .any(|r| r.kind == RelationKind::TranslationInto)
        {
            Some("original")
        } else {
            None
        }
    }
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("static selector must be valid")
}

/// Parses an AO3 download. Returns `None` when the document is not an AO3 download.
pub fn parse(html: &str) -> Option<Ao3Work> {
    let doc = Html::parse_document(html);

    let (work_id, url) = parse_work_link(&doc);
    work_id.as_ref()?;

    let tags = parse_tag_map(&doc);
    let related = parse_related(&doc);
    let language_label = get(&tags, "Language").and_then(|v| v.into_iter().next());

    Some(Ao3Work {
        work_id,
        url,
        title: parse_title(&doc),
        author: parse_author(&doc),
        summary: parse_summary(&doc),
        language_label: language_label.clone(),
        language: language_label
            .as_deref()
            .and_then(language_code)
            .map(str::to_owned),
        tags: Tags {
            rating: get(&tags, "Rating").unwrap_or_default(),
            warnings: get(&tags, "Archive Warning").unwrap_or_default(),
            category: get(&tags, "Category").unwrap_or_default(),
            fandoms: get(&tags, "Fandom").unwrap_or_default(),
            relationships: get(&tags, "Relationship").unwrap_or_default(),
            characters: get(&tags, "Characters").unwrap_or_default(),
            additional: get(&tags, "Additional Tags").unwrap_or_default(),
        },
        stats: parse_stats(get(&tags, "Stats").unwrap_or_default().first()),
        collections: get(&tags, "Collections").unwrap_or_default(),
        related,
    })
}

/// Pulls the work id and canonical URL out of the preface message.
fn parse_work_link(doc: &Html) -> (Option<String>, Option<String>) {
    let message = selector("p.message");
    let anchor = selector("a[href]");

    for p in doc.select(&message) {
        for a in p.select(&anchor) {
            if let Some(href) = a.value().attr("href") {
                if let Some(id) = work_id_from_url(href) {
                    return (Some(id), Some(href.to_owned()));
                }
            }
        }
    }
    (None, None)
}

/// Extracts the numeric id from an AO3 work URL.
pub fn work_id_from_url(url: &str) -> Option<String> {
    let idx = url.find("/works/")?;
    let rest = &url[idx + "/works/".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// The work title, from `div.meta > h1`.
///
/// Explicitly not from `<title>`: that is `Title - Author - Fandom` and cannot be split
/// reliably when the title itself contains " - ".
fn parse_title(doc: &Html) -> Option<String> {
    let h1_in_meta = selector("div.meta h1");
    if let Some(el) = doc.select(&h1_in_meta).next() {
        let t = clean(&el.text().collect::<String>());
        if !t.is_empty() {
            return Some(t);
        }
    }
    // Fallback: first h1 anywhere.
    let h1 = selector("h1");
    doc.select(&h1)
        .next()
        .map(|el| clean(&el.text().collect::<String>()))
        .filter(|t| !t.is_empty())
}

fn parse_author(doc: &Html) -> Option<String> {
    let byline_author = selector("div.byline a[rel=author]");
    if let Some(el) = doc.select(&byline_author).next() {
        let a = clean(&el.text().collect::<String>());
        if !a.is_empty() {
            return Some(a);
        }
    }
    let any_author = selector("a[rel=author]");
    doc.select(&any_author)
        .next()
        .map(|el| clean(&el.text().collect::<String>()))
        .filter(|a| !a.is_empty())
}

/// Reads `dl.tags` into a label -> values map, pairing each `dt` with the `dd` rows
/// that follow it. Matching by label rather than position is what keeps a work with
/// missing rows from shifting every later field.
fn parse_tag_map(doc: &Html) -> Vec<(String, Vec<String>)> {
    use scraper::node::Node;

    let dl_sel = selector("dl.tags");
    let mut out: Vec<(String, Vec<String>)> = Vec::new();

    let Some(dl) = doc.select(&dl_sel).next() else {
        return out;
    };

    let mut current: Option<String> = None;
    for child in dl.children() {
        let Some(el) = ElementRef::wrap(child) else {
            continue;
        };
        match el.value().name() {
            "dt" => {
                let label = clean(&el.text().collect::<String>());
                let label = label.trim_end_matches(':').trim().to_owned();
                current = Some(label.clone());
                out.push((label, Vec::new()));
            }
            "dd" => {
                let Some(label) = current.clone() else {
                    continue;
                };
                let values = dd_values(&el);
                if let Some(entry) = out.iter_mut().find(|(l, _)| *l == label) {
                    entry.1.extend(values);
                }
            }
            _ => {
                // A non-dt/dd child (e.g. the nested ul of related works) ends the run.
                if matches!(el.value().name(), "ul" | "ol" | "div") {
                    current = None;
                }
            }
        }
        let _ = Node::Text;
    }
    out
}

/// Values of one `dd`: its link texts when it has links (tag rows), else its text.
fn dd_values(dd: &ElementRef<'_>) -> Vec<String> {
    let a_sel = selector("a");
    let links: Vec<String> = dd
        .select(&a_sel)
        .map(|a| clean(&a.text().collect::<String>()))
        .filter(|s| !s.is_empty())
        .collect();
    if !links.is_empty() {
        return links;
    }
    let text = clean(&dd.text().collect::<String>());
    if text.is_empty() {
        Vec::new()
    } else {
        vec![text]
    }
}

fn get(map: &[(String, Vec<String>)], label: &str) -> Option<Vec<String>> {
    map.iter()
        .find(|(l, _)| l.eq_ignore_ascii_case(label))
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// Parses the free-text `Stats:` row.
///
/// The value is prose, not structured markup:
/// `Published: 2018-06-09 Completed: 2018-08-29 Words: 94,736 Chapters: 13/13`
/// `Words:` may carry a thousands separator and a trailing `+` when the chapter count
/// is unknown, so the number is extracted by scanning rather than by splitting on it.
fn parse_stats(text: Option<&String>) -> Stats {
    let Some(text) = text else {
        return Stats {
            published: None,
            completed: None,
            words: None,
            chapters: None,
        };
    };

    Stats {
        published: field_date(text, "Published:"),
        completed: field_date(text, "Completed:"),
        words: field_u64(text, "Words:"),
        chapters: field_value(text, "Chapters:"),
    }
}

/// Reads `Label: value` where value runs until the next known label or end of text.
fn field_value(text: &str, label: &str) -> Option<String> {
    const LABELS: [&str; 4] = ["Published:", "Completed:", "Words:", "Chapters:"];
    let start = text.find(label)? + label.len();
    let rest = &text[start..];
    let end = LABELS
        .iter()
        .filter(|l| **l != label)
        .filter_map(|l| rest.find(l))
        .min()
        .unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn field_date(text: &str, label: &str) -> Option<DateValue> {
    DateValue::parse_loose(&field_value(text, label)?, false)
}

fn field_u64(text: &str, label: &str) -> Option<u64> {
    let value = field_value(text, label)?;
    // Strip the thousands separator and any trailing '+' marker.
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The summary, taken from the `blockquote.userstuff` that follows the `Summary` label.
fn parse_summary(doc: &Html) -> Option<String> {
    let p_sel = selector("div.meta p");
    let quote_sel = selector("blockquote.userstuff");

    for p in doc.select(&p_sel) {
        let label = clean(&p.text().collect::<String>());
        if !label.eq_ignore_ascii_case("summary") {
            continue;
        }
        // Walk following siblings to find the summary blockquote, so the Notes
        // blockquote further down is never picked up by mistake.
        let mut node = p.next_sibling();
        while let Some(n) = node {
            if let Some(el) = ElementRef::wrap(n) {
                if el.value().name() == "blockquote" {
                    return Some(text::html_to_text(&el.html()))
                        .map(|s| text::collapse_blank_lines(&s))
                        .filter(|s| !s.is_empty());
                }
                // Another label means the summary was absent (it is optional).
                if el.value().name() == "p" {
                    break;
                }
            }
            node = n.next_sibling();
        }
        let _ = &quote_sel;
    }
    None
}

/// Parses the related-works list, which is the `ul` that follows `dl.tags` inside the
/// preface section. Scoping matters: prose elsewhere can contain "inspired by".
fn parse_related(doc: &Html) -> Vec<RelatedWork> {
    let dl_sel = selector("dl.tags");
    let li_sel = selector("li");

    let Some(dl) = doc.select(&dl_sel).next() else {
        return Vec::new();
    };

    let mut node = dl.next_sibling();
    while let Some(n) = node {
        if let Some(el) = ElementRef::wrap(n) {
            match el.value().name() {
                "ul" | "ol" => {
                    let mut out = Vec::new();
                    for li in el.select(&li_sel) {
                        if let Some(rel) = parse_related_item(&li) {
                            out.push(rel);
                        }
                    }
                    return out;
                }
                // Stop at the chapter body so document prose is never scanned.
                "div" if el.value().attr("id") == Some("chapters") => return Vec::new(),
                "div" => {}
                _ => {}
            }
        }
        node = n.next_sibling();
    }
    Vec::new()
}

/// Parses one `<li>` of the related-works list.
fn parse_related_item(li: &ElementRef<'_>) -> Option<RelatedWork> {
    let a_sel = selector("a[href]");

    // Take the all-text form BEFORE looking at links, to classify the relation.
    let full_text = clean(&li.text().collect::<String>());

    let mut work_id = None;
    let mut url = None;
    let mut title = None;
    let mut author = None;

    for a in li.select(&a_sel) {
        let href = a.value().attr("href").unwrap_or_default();
        let text_value = clean(&a.text().collect::<String>());
        let is_author = a.value().attr("rel") == Some("author");

        if let Some(id) = work_id_from_url(href) {
            if work_id.is_none() {
                work_id = Some(id);
                url = Some(href.to_owned());
                title = Some(text_value);
            }
        } else if is_author && author.is_none() {
            author = Some(text_value);
        }
    }

    // Language of the translation, marked up as <span lang="zh">中文-普通话 國語</span>.
    let span_sel = selector("span[lang]");
    let (language, language_label) = li
        .select(&span_sel)
        .next()
        .and_then(|s| {
            s.value()
                .attr("lang")
                .map(|l| (l.to_owned(), clean(&s.text().collect::<String>())))
        })
        .map(|(code, label)| (Some(code), Some(label)))
        .unwrap_or((None, None));

    let kind = classify_relation(&full_text)?;

    Some(RelatedWork {
        kind,
        work_id,
        url,
        title,
        author,
        language,
        language_label,
    })
}

/// Maps a related-works label to a relation kind.
///
/// Only the two translation phrasings are auto-paired. "Inspired by" and remixes are
/// recorded as relations but never drive automatic pairing, because being inspired by a
/// work is not the same claim as being a translation of it.
fn classify_relation(text: &str) -> Option<RelationKind> {
    let lower = text.trim().to_ascii_lowercase();
    if lower.starts_with("a translation of") {
        return Some(RelationKind::TranslationOf);
    }
    if lower.starts_with("translation into") && lower.contains("available") {
        return Some(RelationKind::TranslationInto);
    }
    if lower.starts_with("a remix of") {
        return Some(RelationKind::RemixOf);
    }
    if lower.starts_with("inspired by") || lower.starts_with("works inspired by") {
        return Some(RelationKind::RelatedTo);
    }
    None
}

/// Collapses whitespace and trims, so text taken from markup compares predictably.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where a relation was discovered, so hand-made links stay distinguishable from
/// AO3's own statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationOrigin {
    /// Stated by AO3 in the downloaded HTML.
    Ao3Html,
    /// Linked by the user in the application.
    Manual,
}

impl RelationOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationOrigin::Ao3Html => "ao3_html",
            RelationOrigin::Manual => "manual",
        }
    }
}

/// One relation to store. `from` and `to` are AO3 work ids.
///
/// Symmetric pairs are emitted with the smaller id first, mirroring the
/// `CHECK (work_a < work_b)` constraint, so the same pair never lands in the table
/// twice in opposite orders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedRelation {
    pub from: String,
    pub to: String,
    pub kind: RelationKind,
    pub origin: RelationOrigin,
}

/// A relation whose partner is known by id but not yet in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRelation {
    /// The id of the work that is missing.
    pub external_id: String,
    pub from_work_id: String,
    pub kind: RelationKind,
    pub language: Option<String>,
    pub language_label: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// What to persist after importing a batch of works.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationPlan {
    /// Both sides are present, so the pair can be stored immediately.
    pub resolved: Vec<PlannedRelation>,
    /// Only one side is present; remembered so the pairing completes by itself when
    /// the other file is imported.
    pub pending: Vec<PendingRelation>,
}

/// Works out which relations can be stored now and which must wait.
///
/// `known` holds the AO3 work ids that exist in the library after this import batch.
/// Only translation relations become explicit pairs here: translations are stated from
/// both sides by AO3, whereas "inspired by" and remixes are kept as links but are
/// asymmetric claims and so are not resolved into pairs.
///
/// A resolved pair always carries [`RelationKind::TranslationOf`], discarding which of
/// the two files revealed it. The store's canonical form is `(smaller_id, larger_id)`,
/// which is direction-free, so a pair found from the original ("Translation into zh
/// available") and from the translation ("A translation of") must normalise to the same
/// value — otherwise the pair would be inserted twice and de-duplication would miss it.
pub fn plan_relations(works: &[Ao3Work], known: &[String]) -> RelationPlan {
    let mut plan = RelationPlan::default();

    for work in works {
        let Some(self_id) = work.work_id.as_deref() else {
            continue;
        };
        for rel in &work.related {
            let Some(other_id) = rel.work_id.as_deref() else {
                continue;
            };

            match rel.kind {
                // "A translation of X": this work is the translation.
                RelationKind::TranslationOf => {
                    if let (Some(a), Some(b)) =
                        (min_str(self_id, other_id), max_str(self_id, other_id))
                    {
                        plan.resolved.push(PlannedRelation {
                            from: a.to_owned(),
                            to: b.to_owned(),
                            kind: RelationKind::TranslationOf,
                            origin: RelationOrigin::Ao3Html,
                        });
                    }
                }
                // "Translation into X available:": the linked work is the translation.
                RelationKind::TranslationInto => {
                    if known.iter().any(|k| k == other_id) {
                        if let (Some(a), Some(b)) =
                            (min_str(self_id, other_id), max_str(self_id, other_id))
                        {
                            plan.resolved.push(PlannedRelation {
                                from: a.to_owned(),
                                to: b.to_owned(),
                                kind: RelationKind::TranslationOf,
                                origin: RelationOrigin::Ao3Html,
                            });
                        }
                    } else {
                        plan.pending.push(PendingRelation {
                            external_id: other_id.to_owned(),
                            from_work_id: self_id.to_owned(),
                            kind: rel.kind,
                            language: rel.language.clone(),
                            language_label: rel.language_label.clone(),
                            title: rel.title.clone(),
                            author: rel.author.clone(),
                        });
                    }
                }
                // Recorded, but not resolved into a pair: these are one-directional.
                RelationKind::RemixOf | RelationKind::RelatedTo => {}
            }
        }
    }

    // A pair can be discovered twice (each file states it). De-duplicate so the same
    // relation is not inserted twice. Sorting uses the same numeric id order as the
    // stored form, so equal pairs are guaranteed to end up adjacent.
    plan.resolved
        .sort_by(|a, b| numeric_cmp(&a.from, &b.from).then_with(|| numeric_cmp(&a.to, &b.to)));
    plan.resolved
        .dedup_by(|a, b| a.from == b.from && a.to == b.to);
    plan.pending.sort_by(|a, b| {
        numeric_cmp(&a.external_id, &b.external_id)
            .then_with(|| numeric_cmp(&a.from_work_id, &b.from_work_id))
    });
    plan.pending
        .dedup_by(|a, b| a.external_id == b.external_id && a.from_work_id == b.from_work_id);

    plan
}

/// Orders two AO3 work ids the way SQLite does for `CHECK (work_a < work_b)`:
/// **numerically**, not as strings.
///
/// This must match the schema's comparison exactly. Compared as strings, `"14885858"`
/// sorts before `"7929115"` because `'1' < '7'`, even though 7929115 is the smaller
/// number — so string ordering would store the pair reversed relative to the CHECK and
/// the insert would be rejected. It would also defeat de-duplication, since the same
/// pair could be recorded in two different orders.
fn min_str<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    if a == b {
        return None; // a work cannot be related to itself
    }
    Some(if numeric_cmp(a, b).is_le() { a } else { b })
}

fn max_str<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    if a == b {
        return None;
    }
    Some(if numeric_cmp(a, b).is_ge() { a } else { b })
}

/// Compares decimal id strings by value. Non-numeric input falls back to string order,
/// which keeps the function total rather than panicking on unexpected data.
fn numeric_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (na, nb) = (a.parse::<u128>(), b.parse::<u128>());
    match (na, nb) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Maps AO3's language display label to an ISO 639-1 code.
///
/// AO3 states only a human-readable label, so a table is unavoidable. It is deliberately
/// static rather than scraped, so the app stays offline and cannot be broken by a change
/// to AO3's language list; the mapping covers AO3's prominent languages and returns
/// `None` for anything else, leaving the original label for display.
pub fn language_code(label: &str) -> Option<&'static str> {
    let l = label.trim();
    // AO3 labels Chinese variants as "中文-普通话 國語", "中文-粵語 廣東話", etc.
    if l.starts_with("中文") {
        return Some("zh");
    }
    Some(match l {
        "English" => "en",
        "Deutsch" => "de",
        "Français" => "fr",
        "Español" => "es",
        "Italiano" => "it",
        "Português brasileiro" | "Português" => "pt",
        "Русский" => "ru",
        "日本語" => "ja",
        "한국어" => "ko",
        "Bahasa Indonesia" => "id",
        "Bahasa Melayu" => "ms",
        "Tiếng Việt" => "vi",
        "ไทย" => "th",
        "Polski" => "pl",
        "Nederlands" => "nl",
        "Svenska" => "sv",
        "Norsk" => "no",
        "Dansk" => "da",
        "Suomi" => "fi",
        "Čeština" => "cs",
        "Slovenčina" => "sk",
        "Magyar" => "hu",
        "Română" => "ro",
        "Български" => "bg",
        "Українська" => "uk",
        "Ελληνικά" => "el",
        "Türkçe" => "tr",
        "العربية" => "ar",
        "עברית" => "he",
        "हिन्दी" => "hi",
        "فارسی" => "fa",
        "Català" => "ca",
        "Galego" => "gl",
        "Latin" => "la",
        "Esperanto" => "eo",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_work_id_from_urls() {
        assert_eq!(
            work_id_from_url("https://archiveofourown.org/works/91520771"),
            Some("91520771".into())
        );
        assert_eq!(
            work_id_from_url("https://archiveofourown.org/works/7929115?view_adult=true"),
            Some("7929115".into())
        );
        assert_eq!(
            work_id_from_url("https://archiveofourown.org/tags/Fluff"),
            None
        );
        assert_eq!(work_id_from_url(""), None);
    }

    #[test]
    fn classifies_only_translations_for_pairing() {
        assert_eq!(
            classify_relation("A translation of One, two, three by Someone"),
            Some(RelationKind::TranslationOf)
        );
        assert_eq!(
            classify_relation(
                "Translation into 中文-普通话 國語 available: 一，二，三 by lisabart"
            ),
            Some(RelationKind::TranslationInto)
        );
        assert_eq!(
            classify_relation("A remix of Something"),
            Some(RelationKind::RemixOf)
        );
        assert_eq!(
            classify_relation("Inspired by Another"),
            Some(RelationKind::RelatedTo)
        );
        // Prose that merely mentions the phrase must not classify as a relation.
        assert_eq!(
            classify_relation("This chapter was inspired by a reader"),
            None
        );
        assert_eq!(classify_relation("Translation into German!"), None);
    }

    #[test]
    fn maps_ao3_language_labels_to_iso_codes() {
        assert_eq!(language_code("English"), Some("en"));
        assert_eq!(language_code("中文-普通话 國語"), Some("zh"));
        assert_eq!(language_code("中文-粵語 廣東話"), Some("zh"));
        assert_eq!(language_code("Deutsch"), Some("de"));
        assert_eq!(language_code("日本語"), Some("ja"));
        // Unknown labels return None rather than a wrong guess.
        assert_eq!(language_code("Klingon"), None);
    }

    #[test]
    fn parses_stats_prose() {
        let s = parse_stats(Some(
            &"Published: 2018-06-09 Completed: 2018-08-29 Words: 94,736 Chapters: 13/13"
                .to_string(),
        ));
        assert_eq!(
            s.words,
            Some(94_736),
            "thousands separators must be stripped"
        );
        assert_eq!(s.published.as_ref().unwrap().display(), "2018-06-09");
        assert_eq!(s.completed.as_ref().unwrap().display(), "2018-08-29");
        assert_eq!(s.chapters.as_deref(), Some("13/13"));
    }

    #[test]
    fn parses_stats_with_unknown_chapter_count() {
        // AO3 appends '+' to the word count when the chapter total is unknown.
        let s = parse_stats(Some(
            &"Published: 2020-01-01 Words: 1,234,567+ Chapters: 3/?".to_string(),
        ));
        assert_eq!(s.words, Some(1_234_567));
        assert_eq!(s.published.as_ref().unwrap().display(), "2020-01-01");
        assert!(s.completed.is_none(), "absent Completed must stay absent");
    }

    #[test]
    fn parses_stats_when_nothing_but_date_is_present() {
        let s = parse_stats(Some(&"Published: 2016-09-01".to_string()));
        assert_eq!(s.published.as_ref().unwrap().display(), "2016-09-01");
        assert!(s.words.is_none());
        assert!(s.chapters.is_none());
    }

    #[test]
    fn empty_stats_yields_no_values() {
        let s = parse_stats(None);
        assert!(s.published.is_none() && s.words.is_none());
    }

    #[test]
    fn rejects_html_that_is_not_an_ao3_download() {
        assert!(parse("<html><body><p>hello</p></body></html>").is_none());
        // Mentions AO3 and even a work URL, but has none of the AO3 structure.
        let fake =
            "<html><body><a href='https://archiveofourown.org/works/123'>x</a></body></html>";
        assert!(parse(fake).is_none());
    }

    /// A minimal but structurally faithful AO3 download, used to cover the traps
    /// (missing Relationship row, title containing " - ", "inspired by" in prose).
    fn synthetic(extra_dl: &str, body_prose: &str, related: &str) -> String {
        format!(
            r#"<!DOCTYPE html><html><head><title>Real - Title - Fandom (TV)</title></head><body>
<div id="preface">
  <p class="message">Posted on AO3 at <a href="https://archiveofourown.org/works/555">link</a>.</p>
  <div class="meta">
    <dl class="tags">
      <dt>Rating:</dt><dd><a href="/tags/G">General</a></dd>
      {extra_dl}
      <dt>Language:</dt><dd>English</dd>
      <dt>Stats:</dt><dd>Published: 2021-02-03 Words: 5,000 Chapters: 1/1</dd>
    </dl>
    {related}
    <h1>Real - Title</h1>
    <div class="byline">by <a rel="author" href="/users/x">Someone</a></div>
    <p>Summary</p>
    <blockquote class="userstuff"><p>A summary.</p></blockquote>
    <p>Notes</p>
    <blockquote class="userstuff"><p>Not the summary.</p></blockquote>
  </div>
</div>
<div id="chapters" class="userstuff"><p>{body_prose}</p></div>
</body></html>"#
        )
    }

    #[test]
    fn title_comes_from_h1_not_the_title_tag() {
        // The <title> tag is "Real - Title - Fandom (TV)"; a naive split would yield
        // "Real", losing half the title.
        let html = synthetic("", "", "");
        let w = parse(&html).expect("should parse");
        assert_eq!(w.title.as_deref(), Some("Real - Title"));
    }

    #[test]
    fn absent_optional_rows_do_not_shift_later_fields() {
        // No Relationship / Characters / Additional Tags rows at all.
        let html = synthetic("", "", "");
        let w = parse(&html).expect("should parse");
        assert!(w.tags.relationships.is_empty());
        assert!(w.tags.additional.is_empty());
        assert_eq!(
            w.author.as_deref(),
            Some("Someone"),
            "author must survive missing rows"
        );
        assert_eq!(
            w.language.as_deref(),
            Some("en"),
            "language must survive missing rows"
        );
        assert_eq!(
            w.stats.words,
            Some(5_000),
            "stats must survive missing rows"
        );
        assert_eq!(
            w.tags.rating,
            vec!["General"],
            "rating row still reads correctly"
        );
    }

    #[test]
    fn summary_is_the_blockquote_after_the_label_not_the_notes() {
        let html = synthetic("", "", "");
        let w = parse(&html).expect("should parse");
        assert_eq!(w.summary.as_deref(), Some("A summary."));
    }

    #[test]
    fn prose_inspired_by_does_not_create_a_relation() {
        let html = synthetic("", "This chapter was inspired by a kind reader.", "");
        let w = parse(&html).expect("should parse");
        assert!(
            w.related.is_empty(),
            "document prose must not fabricate relations: {:?}",
            w.related
        );
    }

    #[test]
    fn translation_relations_are_read_from_the_list_after_the_tag_list() {
        let related = r#"<ul>
            <li>Translation into <span lang="zh">中文-普通话 國語</span> available:
                <a href="https://archiveofourown.org/works/14885858">一，二，三</a> by
                <a rel="author" href="/users/lisabart">lisabart</a></li>
            <li>Translation into <span lang="de">Deutsch</span> available:
                <a href="https://archiveofourown.org/works/81444271">Übersetzung</a> by
                <a rel="author" href="/users/b">Biest1987</a></li>
        </ul>"#;
        let html = synthetic("", "", related);
        let w = parse(&html).expect("should parse");

        assert_eq!(w.related.len(), 2);
        assert_eq!(w.role(), Some("original"));

        let zh = &w.related[0];
        assert_eq!(zh.kind, RelationKind::TranslationInto);
        assert_eq!(zh.work_id.as_deref(), Some("14885858"));
        assert_eq!(
            zh.language.as_deref(),
            Some("zh"),
            "lang attribute gives the code"
        );
        assert_eq!(zh.language_label.as_deref(), Some("中文-普通话 國語"));
        assert_eq!(zh.title.as_deref(), Some("一，二，三"));
        assert_eq!(zh.author.as_deref(), Some("lisabart"));
        assert!(zh.kind.is_translation());
    }

    #[test]
    fn a_translation_work_points_back_at_its_original() {
        let related = r#"<ul><li>A translation of
            <a href="https://archiveofourown.org/works/7929115">One, two, three</a> by
            <a rel="author" href="/users/s">Severus_divides_into_H</a></li></ul>"#;
        let html = synthetic("", "", related);
        let w = parse(&html).expect("should parse");

        assert_eq!(w.role(), Some("translation"));
        let r = &w.related[0];
        assert_eq!(r.kind, RelationKind::TranslationOf);
        assert_eq!(r.work_id.as_deref(), Some("7929115"));
        assert_eq!(
            r.language, None,
            "the original's language is not marked up here"
        );
    }

    /// Two works that state the same pair from opposite sides, as AO3 downloads do.
    fn pair() -> (Ao3Work, Ao3Work) {
        let original = parse(&synthetic(
            "",
            "",
            r#"<ul><li>Translation into <span lang="zh">中文-普通话 國語</span> available:
               <a href="https://archiveofourown.org/works/222">译文</a> by
               <a rel="author" href="/users/t">translator</a></li></ul>"#,
        ))
        .map(|mut w| {
            w.work_id = Some("111".into());
            w
        })
        .unwrap();

        let translation = parse(&synthetic(
            "",
            "",
            r#"<ul><li>A translation of
               <a href="https://archiveofourown.org/works/111">Original</a> by
               <a rel="author" href="/users/a">author</a></li></ul>"#,
        ))
        .map(|mut w| {
            w.work_id = Some("222".into());
            w
        })
        .unwrap();

        (original, translation)
    }

    #[test]
    fn pairing_resolves_when_both_sides_are_imported() {
        let (original, translation) = pair();
        let plan = plan_relations(&[original, translation], &["111".into(), "222".into()]);

        assert_eq!(
            plan.resolved.len(),
            1,
            "the pair is stored exactly once: {:?}",
            plan
        );
        assert_eq!(plan.resolved[0].from, "111");
        assert_eq!(plan.resolved[0].to, "222");
        assert_eq!(plan.resolved[0].kind, RelationKind::TranslationOf);
        assert_eq!(plan.resolved[0].origin, RelationOrigin::Ao3Html);
        assert!(plan.pending.is_empty());
    }

    #[test]
    fn pairing_is_pending_when_only_the_original_is_imported() {
        let (original, _) = pair();
        let plan = plan_relations(&[original], &["111".into()]);

        assert!(
            plan.resolved.is_empty(),
            "one side alone cannot form a pair"
        );
        assert_eq!(plan.pending.len(), 1);
        let p = &plan.pending[0];
        assert_eq!(
            p.external_id, "222",
            "the missing translation is remembered by id"
        );
        assert_eq!(p.from_work_id, "111");
        assert_eq!(
            p.title.as_deref(),
            Some("译文"),
            "so it can be shown as a suggestion"
        );
        assert_eq!(p.language.as_deref(), Some("zh"));
    }

    #[test]
    fn pairing_resolves_from_the_translation_side_alone() {
        // Importing only the translation must still record the pair, because the
        // "A translation of" link names the original directly.
        let (_, translation) = pair();
        let plan = plan_relations(&[translation], &["222".into()]);

        assert_eq!(plan.resolved.len(), 1);
        assert_eq!(plan.resolved[0].from, "111", "smaller id is stored first");
        assert_eq!(plan.resolved[0].to, "222");
    }

    #[test]
    fn pairing_never_exceeds_the_available_pending_side() {
        // The original's list mentions a German translation that is not imported;
        // the Chinese one is. Only the imported one may resolve.
        let original = parse(&synthetic(
            "",
            "",
            r#"<ul>
               <li>Translation into <span lang="zh">中文-普通话 國語</span> available:
                 <a href="https://archiveofourown.org/works/222">中</a></li>
               <li>Translation into <span lang="de">Deutsch</span> available:
                 <a href="https://archiveofourown.org/works/333">德</a></li>
            </ul>"#,
        ))
        .map(|mut w| {
            w.work_id = Some("111".into());
            w
        })
        .unwrap();

        let plan = plan_relations(&[original], &["111".into(), "222".into()]);

        assert_eq!(plan.resolved.len(), 1);
        assert_eq!(plan.resolved[0].to, "222");
        assert_eq!(
            plan.pending.len(),
            1,
            "the un-imported German one stays pending"
        );
        assert_eq!(plan.pending[0].external_id, "333");
    }

    #[test]
    fn pending_relations_are_deduplicated() {
        let (original, _) = pair();
        let plan = plan_relations(&[original.clone(), original], &["111".into()]);
        assert_eq!(
            plan.pending.len(),
            1,
            "repeated imports must not duplicate suggestions"
        );
    }

    #[test]
    fn a_work_cannot_be_related_to_itself() {
        let html = synthetic(
            "",
            "",
            r#"<ul><li>A translation of
               <a href="https://archiveofourown.org/works/111">Self</a></li></ul>"#,
        );
        let work = parse(&html)
            .map(|mut w| {
                w.work_id = Some("111".into());
                w
            })
            .unwrap();
        let plan = plan_relations(&[work], &["111".into()]);
        assert!(plan.resolved.is_empty() && plan.pending.is_empty());
    }

    #[test]
    fn inspired_by_relations_do_not_create_pairs() {
        let html = synthetic(
            "",
            "",
            r#"<ul><li>Inspired by <a href="https://archiveofourown.org/works/777">That one</a></li></ul>"#,
        );
        let work = parse(&html)
            .map(|mut w| {
                w.work_id = Some("111".into());
                w
            })
            .unwrap();
        let plan = plan_relations(&[work], &["111".into(), "777".into()]);
        assert!(plan.resolved.is_empty(), "inspiration is not translation");
        assert!(plan.pending.is_empty());
    }

    #[test]
    fn pair_ordering_is_numeric_not_lexicographic() {
        // Regression guard. The real pair is 7929115 (original) and 14885858
        // (translation). As strings "14885858" < "7929115" because '1' < '7', which
        // would store the pair reversed relative to CHECK (work_a < work_b) and make
        // the INSERT fail. Ordering must be numeric, matching SQLite's integer compare.
        assert!(min_str("14885858", "7929115") == Some("7929115"));
        assert!(max_str("14885858", "7929115") == Some("14885858"));
        assert_eq!(
            numeric_cmp("14885858", "7929115"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(numeric_cmp("9", "10"), std::cmp::Ordering::Less);
        assert_eq!(numeric_cmp("7929115", "7929115"), std::cmp::Ordering::Equal);
        // Non-numeric input must not panic; it falls back to string order.
        assert_eq!(numeric_cmp("abc", "abd"), std::cmp::Ordering::Less);
    }

    #[test]
    fn both_directions_of_the_real_pair_resolve_to_the_same_stored_form() {
        // The original file and the translation file describe the same pair, from
        // opposite sides, with ids of different digit length. Both must normalise to
        // (smaller, larger) or the pair would be stored twice.
        let original = parse(&synthetic(
            "",
            "",
            r#"<ul><li>Translation into <span lang="zh">中文-普通话 國語</span> available:
               <a href="https://archiveofourown.org/works/14885858">一，二，三</a></li></ul>"#,
        ))
        .map(|mut w| {
            w.work_id = Some("7929115".into());
            w
        })
        .unwrap();

        let translation = parse(&synthetic(
            "",
            "",
            r#"<ul><li>A translation of
               <a href="https://archiveofourown.org/works/7929115">One, two, three</a></li></ul>"#,
        ))
        .map(|mut w| {
            w.work_id = Some("14885858".into());
            w
        })
        .unwrap();

        let known = vec!["7929115".to_string(), "14885858".to_string()];
        let plan = plan_relations(&[original, translation], &known);

        assert_eq!(plan.resolved.len(), 1, "one pair, not two: {plan:?}");
        assert_eq!(plan.resolved[0].from, "7929115");
        assert_eq!(plan.resolved[0].to, "14885858");
    }
}
