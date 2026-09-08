//! What a section heading means.
//!
//! The role decides how a `*` line below the heading is read, so this table is
//! the central heuristic of the pipeline. The entries come from counting the
//! headings in the English dump: 212,890 level-2 headings over 39,818 distinct
//! strings, with a short head and a very long tail of work titles.
//!
//! English only. A second language gets its own table once its dump has been
//! measured; see `docs/schema.md`.

use crate::model::{SectionRole, SourceHint};

/// Reads the role from the plain text of a heading.
pub fn role_of(heading: &str) -> SectionRole {
    let name = normalise(heading);

    // "Misattributed" contains "attributed", so the longer names come first.
    if name.contains("misattributed") {
        return SectionRole::Misattributed;
    }
    if name.contains("disputed") {
        return SectionRole::Disputed;
    }
    if name.contains("unsourced") || name == "suggestions" {
        return SectionRole::Unsourced;
    }
    if name.starts_with("attributed") {
        return SectionRole::Attributed;
    }
    if name.starts_with("quotes about") || name == "about" || name.starts_with("about ") {
        return SectionRole::QuotesAbout;
    }
    if matches!(
        name.as_str(),
        "quotes" | "quote" | "quotations" | "sourced" | "sourced quotes"
    ) {
        return SectionRole::Quotes;
    }
    if matches!(
        name.as_str(),
        "dialogue" | "dialogues" | "dialog" | "dialogs"
    ) {
        return SectionRole::Dialogue;
    }
    if name.contains("tagline") {
        return SectionRole::Taglines;
    }
    if matches!(name.as_str(), "song lyrics" | "lyrics" | "songs") {
        return SectionRole::SongLyrics;
    }
    if name.ends_with("cast") || name == "cast members" {
        return SectionRole::Cast;
    }
    if name == "episodes" || name == "seasons" || is_season(&name).is_some() {
        return SectionRole::Episodes;
    }
    if name == "see also" || name == "related" {
        return SectionRole::SeeAlso;
    }
    if name.starts_with("external link") || name.starts_with("other project") {
        return SectionRole::ExternalLinks;
    }
    if matches!(
        name.as_str(),
        "references" | "notes" | "footnotes" | "citations"
    ) {
        return SectionRole::References;
    }

    let mut chars = heading.trim().chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_alphanumeric()
    {
        return SectionRole::AlphaBucket(c.to_ascii_uppercase());
    }

    SectionRole::Other(heading.trim().to_string())
}

/// Reads source meaning from the shape of a heading.
///
/// A heading with no hint gives nothing to the source of a quote. That is the
/// case for `Quotes`, `Misattributed`, `Quotes about X` and the single-letter
/// sort buckets.
pub fn source_hint(heading: &str, is_italic: bool) -> Option<SourceHint> {
    let text = heading.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(number) = is_season(&normalise(text)) {
        return Some(SourceHint::Season { number });
    }
    if let Some(period) = as_period(text) {
        return Some(period);
    }
    if let Some(episode) = as_episode(text) {
        return Some(episode);
    }
    if let Some(locator) = as_locator(text) {
        return Some(locator);
    }
    let (title, year) = split_year(text);
    // A work title is either written in italics or followed by a year.
    if is_italic || year.is_some() {
        return Some(SourceHint::Work {
            title: title.to_string(),
            year,
        });
    }
    None
}

fn normalise(heading: &str) -> String {
    heading.trim().trim_end_matches(':').trim().to_lowercase()
}

fn is_season(name: &str) -> Option<u16> {
    let rest = name
        .strip_prefix("season ")
        .or_else(|| name.strip_prefix("series "))?
        .trim();
    rest.parse().ok().or_else(|| word_number(rest))
}

/// `Season One` is as common on Wikiquote as `Season 1`.
fn word_number(word: &str) -> Option<u16> {
    const WORDS: [&str; 12] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve",
    ];
    WORDS
        .iter()
        .position(|w| *w == word)
        .map(|index| index as u16 + 1)
}

/// A heading that names a place inside a work: `Chapter 5`, `Act II`,
/// `Book I`, `Preface`. It is not a work of its own.
fn as_locator(text: &str) -> Option<SourceHint> {
    let text = text.trim();
    let lower = text.to_lowercase();
    for (label, kind) in [
        ("chapter", "chapter"),
        ("ch.", "chapter"),
        ("canto", "chapter"),
        ("act", "act_scene"),
        ("scene", "act_scene"),
        ("part", "part"),
        ("book", "part"),
        ("volume", "part"),
        ("vol.", "part"),
        ("no.", "part"),
    ] {
        if let Some(rest) = lower.strip_prefix(label) {
            let rest = rest.trim();
            let is_number = !rest.is_empty()
                && (rest.chars().all(|c| c.is_ascii_digit())
                    || rest.chars().all(|c| "ivxlc".contains(c))
                    || word_number(rest).is_some());
            if is_number {
                return Some(SourceHint::Locator {
                    kind: kind.to_string(),
                    value: text.to_string(),
                });
            }
        }
    }
    if matches!(
        lower.as_str(),
        "preface" | "prologue" | "epilogue" | "introduction" | "incipit" | "foreword" | "afterword"
    ) {
        return Some(SourceHint::Locator {
            kind: "part".to_string(),
            value: text.to_string(),
        });
    }
    None
}

/// `1790s`, `1997` or `1914-1918`.
fn as_period(text: &str) -> Option<SourceHint> {
    let text = text.trim();
    if let Some(decade) = text.strip_suffix('s')
        && let Ok(year) = decade.parse::<i32>()
        && decade.len() == 4
    {
        return Some(SourceHint::Period {
            from: year,
            to: year + 9,
        });
    }
    if text.len() == 4
        && let Ok(year) = text.parse::<i32>()
    {
        return Some(SourceHint::Period {
            from: year,
            to: year,
        });
    }
    let (from, to) = text.split_once(['-', '\u{2013}'])?;
    let (from, to) = (from.trim().parse().ok()?, to.trim().parse().ok()?);
    if from > to {
        None
    } else {
        Some(SourceHint::Period { from, to })
    }
}

/// `Pilot [1.01]`, `The Cage [1x01]` or a plain `Episode 4`.
fn as_episode(text: &str) -> Option<SourceHint> {
    if let Some(number) = text
        .trim()
        .strip_prefix("Episode ")
        .or_else(|| text.trim().strip_prefix("episode "))
        .map(str::trim)
        && number.chars().all(|c| c.is_ascii_digit())
        && !number.is_empty()
    {
        return Some(SourceHint::Episode {
            title: None,
            code: Some(number.to_string()),
        });
    }
    let open = text.rfind('[')?;
    let close = text.rfind(']')?;
    if close < open {
        return None;
    }
    let code = text[open + 1..close].trim();
    let looks_like_a_code = !code.is_empty()
        && code
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == 'x' || c == '-');
    if !looks_like_a_code {
        return None;
    }
    let title = text[..open].trim();
    Some(SourceHint::Episode {
        title: (!title.is_empty()).then(|| title.to_string()),
        code: Some(code.to_string()),
    })
}

/// Splits `Common Sense (1776)` into its title and its year.
fn split_year(text: &str) -> (&str, Option<u16>) {
    let Some(open) = text.rfind('(') else {
        return (text, None);
    };
    if !text.trim_end().ends_with(')') {
        return (text, None);
    }
    let close = text.rfind(')').unwrap_or(text.len());
    let inside = text[open + 1..close].trim();
    let digits: String = inside.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 4 || !inside.starts_with(|c: char| c.is_ascii_digit() || c.is_alphabetic()) {
        return (text, None);
    }
    match digits.parse() {
        Ok(year) => (text[..open].trim(), Some(year)),
        Err(_) => (text, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_common_roles() {
        assert_eq!(role_of("Quotes"), SectionRole::Quotes);
        assert_eq!(role_of("  quotations "), SectionRole::Quotes);
        assert_eq!(role_of("External links"), SectionRole::ExternalLinks);
        assert_eq!(role_of("Other projects:"), SectionRole::ExternalLinks);
        assert_eq!(role_of("Voice cast"), SectionRole::Cast);
        assert_eq!(role_of("Dialogue"), SectionRole::Dialogue);
        assert_eq!(role_of("Taglines"), SectionRole::Taglines);
        assert_eq!(role_of("Season 3"), SectionRole::Episodes);
    }

    #[test]
    fn tells_attributed_from_misattributed() {
        assert_eq!(role_of("Attributed"), SectionRole::Attributed);
        assert_eq!(role_of("Misattributed"), SectionRole::Misattributed);
        assert_eq!(
            role_of("Misattributed Quotes about Paine"),
            SectionRole::Misattributed
        );
    }

    #[test]
    fn reads_the_quotes_about_forms() {
        assert_eq!(role_of("Quotes about Paine"), SectionRole::QuotesAbout);
        assert_eq!(role_of("About"), SectionRole::QuotesAbout);
        assert_eq!(role_of("About Thomas Paine"), SectionRole::QuotesAbout);
    }

    #[test]
    fn a_single_letter_is_a_sort_bucket() {
        assert_eq!(role_of("S"), SectionRole::AlphaBucket('S'));
        assert_eq!(role_of("z"), SectionRole::AlphaBucket('Z'));
        assert_eq!(role_of("A - F"), SectionRole::Other("A - F".to_string()));
    }

    #[test]
    fn an_unknown_heading_keeps_its_text() {
        assert_eq!(
            role_of("Hoyt's New Cyclopedia"),
            SectionRole::Other("Hoyt's New Cyclopedia".to_string())
        );
    }

    #[test]
    fn a_role_heading_gives_no_source() {
        assert_eq!(source_hint("Quotes", false), None);
        assert_eq!(source_hint("Quotes about Paine", false), None);
        assert_eq!(source_hint("S", false), None);
    }

    #[test]
    fn reads_a_work_title() {
        assert_eq!(
            source_hint("Common Sense (1776)", false),
            Some(SourceHint::Work {
                title: "Common Sense".into(),
                year: Some(1776)
            })
        );
        assert_eq!(
            source_hint("The American Crisis", true),
            Some(SourceHint::Work {
                title: "The American Crisis".into(),
                year: None
            })
        );
    }

    #[test]
    fn reads_a_period() {
        assert_eq!(
            source_hint("1790s", false),
            Some(SourceHint::Period {
                from: 1790,
                to: 1799
            })
        );
        assert_eq!(
            source_hint("1997", false),
            Some(SourceHint::Period {
                from: 1997,
                to: 1997
            })
        );
        assert_eq!(
            source_hint("1914-1918", false),
            Some(SourceHint::Period {
                from: 1914,
                to: 1918
            })
        );
    }

    #[test]
    fn reads_a_season_and_an_episode() {
        assert_eq!(
            source_hint("Season 1", false),
            Some(SourceHint::Season { number: 1 })
        );
        assert_eq!(
            source_hint("Pilot [1.01]", true),
            Some(SourceHint::Episode {
                title: Some("Pilot".into()),
                code: Some("1.01".into())
            })
        );
    }

    #[test]
    fn reads_a_plain_episode_number() {
        assert_eq!(
            source_hint("Episode 4", false),
            Some(SourceHint::Episode {
                title: None,
                code: Some("4".into())
            })
        );
        assert_eq!(source_hint("Episode of a life", false), None);
    }

    #[test]
    fn a_plain_heading_is_not_a_work() {
        assert_eq!(source_hint("Other", false), None);
        assert_eq!(
            source_hint("Discourse to the Theophilanthropists", false),
            None
        );
    }

    #[test]
    fn reads_a_place_inside_a_work() {
        assert_eq!(
            source_hint("Chapter 5", false),
            Some(SourceHint::Locator {
                kind: "chapter".into(),
                value: "Chapter 5".into()
            })
        );
        assert_eq!(
            source_hint("Act II", false),
            Some(SourceHint::Locator {
                kind: "act_scene".into(),
                value: "Act II".into()
            })
        );
        assert_eq!(
            source_hint("Preface", false),
            Some(SourceHint::Locator {
                kind: "part".into(),
                value: "Preface".into()
            })
        );
        // A title that opens with one of those words is still a title.
        assert_eq!(source_hint("Book of Mormon", false), None);
    }

    #[test]
    fn reads_a_season_written_as_a_word() {
        assert_eq!(
            source_hint("Season One", false),
            Some(SourceHint::Season { number: 1 })
        );
        assert_eq!(role_of("Season One"), SectionRole::Episodes);
    }
}
