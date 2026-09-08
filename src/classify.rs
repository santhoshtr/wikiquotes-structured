//! Decides what a page is about.
//!
//! The signal is the category list, which is the only thing on a Wikiquote page
//! that says what kind of subject it has. The rules were checked against the
//! whole English dump; the counts in the tests come from that run.
//!
//! `Unknown` is a normal answer, not a failure. About 15,500 pages carry no
//! category that says anything, and 5,930 carry no category at all. Layer 2
//! treats an unknown page the same way as a theme page, which is what those
//! pages nearly always are.

use crate::model::{Block, Document, PageType, TemplateRef};

/// Returns the page type and the category or template that decided it.
pub fn classify(document: &Document) -> (PageType, Vec<String>) {
    let categories: Vec<String> = document.categories.iter().map(|c| c.to_lowercase()).collect();
    let templates = template_names(document);

    for &(page_type, test) in RULES {
        if let Some(evidence) = test(&categories, &templates, &document.title) {
            return (page_type, vec![evidence]);
        }
    }
    (PageType::Unknown, Vec::new())
}

type Test = fn(&[String], &[String], &str) -> Option<String>;

/// Order matters. An actor page carries film categories, and a page about a
/// film based on a novel carries both. The more specific subject wins.
const RULES: &[(PageType, Test)] = &[
    (PageType::Disambiguation, |_, templates, _| {
        template_named(templates, &["disambig", "disambiguation", "hndis"])
    }),
    (PageType::Placeholder, |_, templates, _| {
        template_named(templates, &["year page placeholder"])
    }),
    (PageType::Person, |categories, _, _| {
        category_where(categories, |c| {
            c == "living people" || c.ends_with(" births") || c.ends_with(" deaths")
        })
    }),
    (PageType::TelevisionSeason, |categories, _, _| {
        category_where(categories, |c| c.ends_with(" seasons"))
    }),
    (PageType::Film, |categories, _, _| {
        category_where(categories, |c| c.ends_with(" film") || c.ends_with(" films"))
    }),
    (PageType::TelevisionSeries, |categories, _, _| {
        category_where(categories, |c| {
            c == "cancelled shows"
                || c.contains("television series")
                || c.contains("television programs")
                || c.contains("web series")
                || c.contains("anime")
        })
    }),
    (PageType::VideoGame, |categories, _, _| {
        category_where(categories, |c| c.contains("video games"))
    }),
    (PageType::MusicalWork, |categories, _, _| {
        category_where(categories, |c| c.ends_with(" songs") || c.ends_with(" albums"))
    }),
    (PageType::LiteraryWork, |categories, _, _| {
        category_where(categories, |c| {
            c.starts_with("works by")
                || matches!(
                    last_word(c),
                    "novels" | "books" | "plays" | "poems" | "essays" | "stories" | "works"
                )
        })
    }),
    (PageType::Proverbs, |categories, _, _| {
        category_where(categories, |c| c.contains("proverbs"))
    }),
    (PageType::Theme, |categories, _, _| {
        category_where(categories, |c| c == "themes" || c == "virtues" || c == "emotions")
    }),
    (PageType::List, |_, _, title| {
        let lower = title.to_lowercase();
        (lower.starts_with("list of") || lower.starts_with("lists of"))
            .then(|| format!("title: {title}"))
    }),
];

fn category_where(categories: &[String], test: impl Fn(&str) -> bool) -> Option<String> {
    categories.iter().find(|c| test(c)).map(|c| format!("category: {c}"))
}

fn template_named(templates: &[String], names: &[&str]) -> Option<String> {
    templates.iter().find(|t| names.contains(&t.as_str())).map(|t| format!("template: {t}"))
}

fn last_word(category: &str) -> &str {
    category.rsplit(' ').next().unwrap_or(category)
}

/// Template names from the lead, where the page-wide markers live.
fn template_names(document: &Document) -> Vec<String> {
    let lead = document.lead.iter().filter_map(|block| match block {
        Block::Template(template) => Some(template),
        _ => None,
    });
    lead.chain(document.templates.iter()).map(|t: &TemplateRef| t.name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn page(categories: &[&str], templates: &[&str], title: &str) -> Document {
        Document {
            wiki_language: "en".into(),
            title: title.into(),
            page_id: 1,
            revision_id: 1,
            timestamp: String::new(),
            redirect_to: None,
            page_type: PageType::Unknown,
            page_type_evidence: Vec::new(),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            templates: templates
                .iter()
                .map(|name| TemplateRef {
                    name: name.to_string(),
                    params: Vec::new(),
                    wikitext: String::new(),
                })
                .collect(),
            interwiki: Vec::new(),
            lead: Vec::new(),
            sections: Vec::new(),
            parse: ParseStats::default(),
        }
    }

    fn kind_of(categories: &[&str], templates: &[&str], title: &str) -> PageType {
        classify(&page(categories, templates, title)).0
    }

    #[test]
    fn reads_a_person_from_a_birth_or_death_category() {
        assert_eq!(kind_of(&["1737 births"], &[], "Thomas Paine"), PageType::Person);
        assert_eq!(kind_of(&["BCE deaths"], &[], "Socrates"), PageType::Person);
        assert_eq!(kind_of(&["Living people"], &[], "Someone"), PageType::Person);
    }

    #[test]
    fn an_actor_is_a_person_not_a_film() {
        let kind = kind_of(&["2020 American films", "Living people"], &[], "Leah Lewis");
        assert_eq!(kind, PageType::Person);
    }

    #[test]
    fn a_film_of_a_novel_is_a_film() {
        assert_eq!(
            kind_of(&["Films based on novels", "2020 American films"], &[], "The Half of It"),
            PageType::Film
        );
    }

    #[test]
    fn reads_the_other_subject_kinds() {
        assert_eq!(kind_of(&["The Simpsons seasons"], &[], "S21"), PageType::TelevisionSeason);
        assert_eq!(kind_of(&["Cancelled shows"], &[], "Show"), PageType::TelevisionSeries);
        assert_eq!(kind_of(&["1990s video games"], &[], "Game"), PageType::VideoGame);
        assert_eq!(kind_of(&["Works by Alexander Pope"], &[], "The Dunciad"), PageType::LiteraryWork);
        assert_eq!(kind_of(&["Latin proverbs"], &[], "Proverb"), PageType::Proverbs);
        assert_eq!(kind_of(&["Themes"], &[], "Warmness"), PageType::Theme);
        assert_eq!(kind_of(&[], &[], "List of last words"), PageType::List);
    }

    #[test]
    fn a_template_marks_a_disambiguation_or_a_placeholder() {
        assert_eq!(kind_of(&[], &["disambig"], "Smith"), PageType::Disambiguation);
        assert_eq!(kind_of(&[], &["year page placeholder"], "1903"), PageType::Placeholder);
    }

    #[test]
    fn an_unknown_page_says_so_and_gives_no_evidence() {
        let (kind, evidence) = classify(&page(&["American bands"], &[], "Steely Dan"));
        assert_eq!(kind, PageType::Unknown);
        assert!(evidence.is_empty());
    }

    #[test]
    fn keeps_the_reason_for_the_answer() {
        let (_, evidence) = classify(&page(&["1737 births"], &[], "Thomas Paine"));
        assert_eq!(evidence, ["category: 1737 births"]);
    }
}
