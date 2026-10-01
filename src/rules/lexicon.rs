//! Embedded phrase data and optional contextual guards shared by rule families.
use crate::diagnostics::Span;
use crate::md::Language;
use crate::md::prose::{Run, marker_if, occurrences};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Guard {
    EndUser,
}

#[derive(Deserialize)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct Entry {
    phrase: String,
    guard: Option<Guard>,
    #[cfg(test)]
    hits: Vec<Example>,
    #[cfg(test)]
    misses: Vec<Example>,
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    text: String,
    // Some rules report a value or link, rather than the triggering phrase.
    target: Option<String>,
}

type Data = BTreeMap<String, BTreeMap<String, Vec<Entry>>>;
static DATA: LazyLock<Data> = LazyLock::new(|| {
    let mut data: Data =
        toml::from_str(include_str!("lexicons/normative.toml")).expect("valid normative lexicon");
    let heuristic: Data =
        toml::from_str(include_str!("lexicons/heuristic.toml")).expect("valid heuristic lexicon");
    data.extend(heuristic);
    data
});

#[derive(Clone, Copy)]
pub(super) struct Phrase<'a> {
    pub text: &'a str,
    pub guard: Option<Guard>,
}
impl Phrase<'_> {
    fn allows(&self, before: &str, after: &str) -> bool {
        !matches!(self.guard, Some(Guard::EndUser)) || !end_user_behavior(before, after)
    }

    pub fn occurrences(&self, text: &str) -> Vec<Span> {
        occurrences(text, self.text)
            .into_iter()
            .filter(|range| self.allows(&text[..range.start], &text[range.end..]))
            .collect()
    }
}

type Phrases = BTreeMap<String, BTreeMap<String, Vec<Phrase<'static>>>>;
static PHRASES: LazyLock<Phrases> = LazyLock::new(|| {
    DATA.iter()
        .map(|(group, languages)| {
            (
                group.clone(),
                languages
                    .iter()
                    .map(|(language, entries)| {
                        (
                            language.clone(),
                            entries
                                .iter()
                                .map(|entry| Phrase {
                                    text: &entry.phrase,
                                    guard: entry.guard,
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
});

pub(super) fn phrases(group: &str, language: Language) -> &'static [Phrase<'static>] {
    let phrases = &PHRASES[group][language.as_str()];
    #[cfg(test)]
    if let Some((start, end)) = tests::selected_range(group, language.as_str()) {
        return &phrases[start..end];
    }
    phrases
}

pub(super) fn marker(runs: &[Run<'_>], phrases: &[Phrase<'_>]) -> Option<Span> {
    phrases
        .iter()
        .filter_map(|phrase| {
            marker_if(runs, &[phrase.text], |before, after| {
                phrase.allows(before, after)
            })
        })
        .min_by_key(|span| (span.start, span.end))
}

/// Product documentation describes its own end users with the same words as a
/// report about the requester: "未经用户授权", "Verify that the user has
/// authorized the app", "用户已授权的应用", "经用户确认后". A condition,
/// negation, requirement, or check earlier in the same clause, or an
/// attributive or temporal clause after the phrase, marks that use. Earlier
/// clauses do not count, so "After the review, the user confirmed" remains a
/// report.
const END_USER_ADJACENT: &[&str] = &["未", "需", "须", "应", "不", "待", "等", "请", "若", "当"];
const END_USER_CLAUSE_PHRASES: &[&str] = &[
    "如果", "一旦", "只有", "除非", "确认", "确保", "检查", "验证", "核实", "必须", "需要", "要求",
    "是否", "判断",
];
const END_USER_CLAUSE_WORDS: &[&str] = &[
    "if", "once", "when", "whenever", "after", "until", "unless", "before", "whether", "verify",
    "ensure", "check", "confirm", "sure", "require", "requires", "required", "must", "should",
    "need", "needs", "only", "wait",
];
const END_USER_AFTER: &[&str] = &["的", "后", "之后", "以后", "时"];
const CLAUSE_BOUNDARIES: &[char] = &[
    ',', ';', ':', '.', '!', '?', '，', '；', '：', '。', '！', '？', '、',
];

fn end_user_behavior(before: &str, after: &str) -> bool {
    let before = before.trim_end();
    let clause = before
        .rfind(CLAUSE_BOUNDARIES)
        .map_or(before, |index| &before[index..]);
    END_USER_ADJACENT.iter().any(|word| before.ends_with(word))
        || END_USER_CLAUSE_PHRASES
            .iter()
            .any(|phrase| clause.contains(phrase))
        || clause
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| END_USER_CLAUSE_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        || END_USER_AFTER
            .iter()
            .any(|word| after.trim_start().starts_with(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, diagnostics::Diagnostic, md};
    use std::cell::Cell;
    use std::collections::BTreeSet;

    // Override only the tested group on this thread. Secondary cues still use
    // their normal data, and integration tests never compile this hook.
    #[derive(Clone, Copy)]
    struct Selection {
        group: &'static str,
        language: &'static str,
        start: usize,
        end: usize,
    }
    thread_local! {
        static SELECTION: Cell<Option<Selection>> = const { Cell::new(None) };
    }

    pub(super) fn selected_range(group: &str, language: &str) -> Option<(usize, usize)> {
        SELECTION
            .get()
            .filter(|selected| selected.group == group && selected.language == language)
            .map(|selected| (selected.start, selected.end))
    }

    fn with_entries<T>(
        group: &'static str,
        language: &'static str,
        start: usize,
        end: usize,
        run: impl FnOnce() -> T,
    ) -> T {
        struct Restore(Option<Selection>);
        impl Drop for Restore {
            fn drop(&mut self) {
                SELECTION.set(self.0);
            }
        }
        let _restore = Restore(SELECTION.replace(Some(Selection {
            group,
            language,
            start,
            end,
        })));
        run()
    }

    fn owner(group: &str) -> &'static str {
        match group {
            "stale" | "constraint" => "STL001",
            "commit" => "STL003",
            "pointer" => "PTR001",
            "source" => "PTR003",
            "rationale" => "RAT002",
            "conversation" => "VOX001",
            "deployment" => "STL002",
            "excluded_heading" => "VOX002",
            "production_heading" | "narration" => "VOX003",
            "evaluation" => "EVD001",
            _ => panic!("unrecognized lexicon group: {group}"),
        }
    }

    fn check(group: &str, language: &str, example: &Example) -> (String, Vec<Diagnostic>) {
        let root = std::env::current_dir().unwrap();
        let config = Config::parse("preview = true", &root).unwrap();
        let source = format!(
            "---\nkind: howto\nlang: {language}\n---\n{}\n",
            example.text
        );
        let document = md::parse(&source).unwrap();
        let enabled = BTreeSet::from([owner(group).to_owned()]);
        let diagnostics = if [
            "deployment",
            "excluded_heading",
            "production_heading",
            "narration",
            "evaluation",
        ]
        .contains(&group)
        {
            crate::rules::heuristic::check(&document, "guide.md", &config, &enabled).diagnostics
        } else {
            crate::rules::normative::check(
                &document,
                "guide.md",
                &root.join("guide.md"),
                &root,
                &config,
                &enabled,
            )
        };
        (source, diagnostics)
    }

    #[test]
    fn every_builtin_phrase_has_executable_intended_use_examples() {
        let mut failures = Vec::new();
        for (group, languages) in DATA.iter() {
            assert_eq!(
                languages.keys().map(String::as_str).collect::<Vec<_>>(),
                ["en", "ja", "zh"]
            );
            for (language, entries) in languages {
                let mut seen = BTreeSet::new();
                for (index, entry) in entries.iter().enumerate() {
                    let label = format!("{group}/{language}/{:?}", entry.phrase);
                    assert!(
                        !entry.phrase.trim().is_empty() && seen.insert(&entry.phrase),
                        "{label}"
                    );
                    assert!(
                        !entry.hits.is_empty() && !entry.misses.is_empty(),
                        "missing examples: {label}"
                    );
                    for (hit, examples) in [(true, &entry.hits), (false, &entry.misses)] {
                        for example in examples {
                            assert!(
                                !occurrences(&example.text, &entry.phrase).is_empty(),
                                "example must exercise its entry: {label}: {}",
                                example.text
                            );
                            let (source, diagnostics) = check(group, language, example);
                            let expected = if group == "constraint" {
                                example.target.as_deref()
                            } else if hit {
                                Some(example.target.as_deref().unwrap_or(&entry.phrase))
                            } else {
                                assert!(
                                    example.target.is_none(),
                                    "misses cannot expect diagnostics: {label}"
                                );
                                None
                            };
                            let matches = |diagnostics: &[Diagnostic]| match expected {
                                None => diagnostics.is_empty(),
                                Some(target) => {
                                    // Overlapping entries can select a shorter phrase. Require the
                                    // diagnostic to overlap this entry/target, not just fire elsewhere.
                                    let targets = occurrences(&source, target);
                                    diagnostics.iter().any(|diagnostic| {
                                        diagnostic.code == owner(group)
                                            && targets.iter().any(|target| {
                                                target.start < diagnostic.byte_range.end
                                                    && diagnostic.byte_range.start < target.end
                                            })
                                    })
                                }
                            };
                            if !matches(&diagnostics) {
                                failures.push(format!("{label} hit={hit}: {}\n  expected {expected:?}; got {diagnostics:?}", example.text));
                            }
                            // A sibling such as 见 must not stand in for 参见.
                            // Run the same owner and span assertion with this
                            // group's candidate as its only available entry.
                            let (_, isolated) =
                                with_entries(group, language, index, index + 1, || {
                                    check(group, language, example)
                                });
                            if !matches(&isolated) {
                                failures.push(format!("isolated {label} hit={hit}: {}\n  expected {expected:?}; got {isolated:?}", example.text));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn overlapping_pointer_cannot_replace_an_unavailable_candidate() {
        let entries = &DATA["pointer"]["zh"];
        let index = entries
            .iter()
            .position(|entry| entry.phrase == "参见")
            .unwrap();
        let example = &entries[index].hits[0];
        assert_eq!(check("pointer", "zh", example).1.len(), 1);
        assert_eq!(
            with_entries("pointer", "zh", index, index + 1, || check(
                "pointer", "zh", example
            ))
            .1
            .len(),
            1
        );
        assert!(
            with_entries("pointer", "zh", index, index, || check(
                "pointer", "zh", example
            ))
            .1
            .is_empty()
        );
        assert_eq!(check("pointer", "zh", example).1.len(), 1);
    }
}
