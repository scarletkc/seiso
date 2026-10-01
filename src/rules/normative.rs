use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use super::lexicon::{self, Guard, Phrase, marker};
use crate::config::{Config, Lexicon};
use crate::diagnostics::{Diagnostic, Span};
use crate::md::prose::{Run, runs};
use crate::md::{BlockKind, Document, Fragment, FragmentKind, Language};
use crate::paths::{local_link_target, normalize};
use regex::Regex;

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>]+").unwrap());
static VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:v?[0-9]+(?:\.[0-9]+)+(?:[-+][a-z0-9.-]+)?|[a-f0-9]{7,40}|[a-z][a-z0-9_.-]*(?:/[a-z0-9_.-]+)*:[a-z0-9_.-]+|[0-9]+)").unwrap()
});
static BOUND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:>=|<=|≥|≤|\d(?:\.\d+)*\+)").unwrap());
static HASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\s:=#]*(?P<hash>[a-fA-F0-9]{7,40})(?:$|[^a-zA-Z0-9_])").unwrap()
});
static FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9_.-])(?:[a-z0-9_@.+-]+[/\\])*[a-z0-9_@+-]+\.(?:rs|py|pyi|js|jsx|ts|tsx|c|h|cc|cpp|hpp|cs|go|java|kt|swift|rb|php|sh|ps1|json|toml|yaml|yml|md|markdown|txt|xml|html|css|sql|proto|vue|svelte)(?:$|[^a-z0-9_.-]|\.(?:$|\s))").unwrap()
});
static SYMBOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+|[A-Za-z][A-Za-z0-9]*_[A-Za-z0-9_]+|[A-Za-z_][A-Za-z0-9_]*\(\)|[A-Z][a-z]+[A-Z][A-Za-z0-9]*|[A-Z][A-Z0-9_]{2,})(?:$|[^\p{L}\p{N}_])").unwrap()
});
static JAPANESE_CHOICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*なぜ.{1,100}(?:を選んだ|を選ぶ|を採用した|を採用する|を使わない|を使うのか)")
        .unwrap()
});

#[derive(Clone, Copy)]
enum Words {
    Stale,
    Constraint,
    Commit,
    Pointer,
    Source,
    Rationale,
    Conversation,
}

fn extensions(words: Words, lexicon: &Lexicon) -> &[String] {
    match words {
        Words::Stale => &lexicon.extend_stale_markers,
        Words::Constraint => &lexicon.extend_constraint_markers,
        Words::Commit => &lexicon.extend_commit_contexts,
        Words::Pointer => &lexicon.extend_pointer_markers,
        Words::Source => &lexicon.extend_source_pointers,
        Words::Rationale => &lexicon.extend_rationale_headings,
        Words::Conversation => &lexicon.extend_conversation_markers,
    }
}

fn words(config: &Config, language: Language, kind: Words) -> Vec<Phrase<'_>> {
    let group = match kind {
        Words::Stale => "stale",
        Words::Constraint => "constraint",
        Words::Commit => "commit",
        Words::Pointer => "pointer",
        Words::Source => "source",
        Words::Rationale => "rationale",
        Words::Conversation => "conversation",
    };
    let mut values = lexicon::phrases(group, language).to_vec();
    if let Some(lexicon) = config.settings.lint.lexicon.get(language.as_str()) {
        values.extend(extensions(kind, lexicon).iter().map(|text| Phrase {
            text,
            guard: matches!(kind, Words::Conversation).then_some(Guard::EndUser),
        }));
    }
    values
}

pub(crate) fn constrained(sentence: &crate::md::Sentence, config: &Config) -> bool {
    let visible = runs(sentence, true);
    marker(
        &visible,
        &words(config, sentence.language, Words::Constraint),
    )
    .is_some()
        || visible.iter().any(|run| BOUND.is_match(&run.text))
}

fn volatile_value(run: &Run<'_>) -> Option<Span> {
    VALUE.find_iter(&run.text).find_map(|value| {
        let identifier = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
        if run.text[..value.start()]
            .chars()
            .next_back()
            .is_some_and(identifier)
            || run.text[value.end()..]
                .chars()
                .next()
                .is_some_and(identifier)
        {
            return None;
        }
        run.span(Span::new(value.start(), value.end()))
    })
}

fn emit(
    document: &Document,
    filename: &str,
    code: &str,
    span: Span,
    message: &str,
    suggestion: &str,
) -> Diagnostic {
    Diagnostic::new(filename, &document.source, code, span, message, suggestion)
}

pub fn check(
    document: &Document,
    filename: &str,
    path: &Path,
    workspace_root: &Path,
    config: &Config,
    enabled: &BTreeSet<String>,
) -> Vec<Diagnostic> {
    let checks = NormativeChecks {
        document,
        filename,
        path,
        workspace_root,
        config,
        enabled,
    };
    let mut diagnostics = Vec::new();
    for sentence in &document.sentences {
        if !config
            .settings
            .lint
            .languages
            .iter()
            .any(|lang| lang == sentence.language.as_str())
        {
            continue;
        }
        let prose = runs(sentence, false);
        let with_code = runs(sentence, true);
        checks.check_stale_values(sentence, &prose, &with_code, &mut diagnostics);
        checks.check_identifiers(sentence, &with_code, &mut diagnostics);
        checks.check_root_pointers(sentence, &prose, &mut diagnostics);
        checks.check_source_pointers(sentence, &prose, &mut diagnostics);
        checks.check_rationale_headings(sentence, &prose, &mut diagnostics);
        checks.check_conversation(sentence, &prose, &mut diagnostics);
    }
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert((diagnostic.code.clone(), diagnostic.byte_range)));
    diagnostics
}

struct NormativeChecks<'a> {
    document: &'a Document,
    filename: &'a str,
    path: &'a Path,
    workspace_root: &'a Path,
    config: &'a Config,
    enabled: &'a BTreeSet<String>,
}

impl NormativeChecks<'_> {
    fn check_stale_values(
        &self,
        sentence: &crate::md::Sentence,
        prose: &[Run<'_>],
        with_code: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("STL001") {
            let stale = marker(prose, &words(self.config, sentence.language, Words::Stale));
            let constraint = marker(
                with_code,
                &words(self.config, sentence.language, Words::Constraint),
            );
            if stale.is_some()
                && constraint.is_none()
                && !with_code.iter().any(|run| BOUND.is_match(&run.text))
                && let Some(span) = with_code.iter().find_map(volatile_value)
            {
                diagnostics.push(emit(
                    self.document,
                    self.filename,
                    "STL001",
                    span,
                    "This sentence pairs a current-state marker with a value that can change.",
                    "Link to the source that owns the value, or state a lasting requirement with its constraint.",
                ));
            }
        }
    }

    fn check_identifiers(
        &self,
        sentence: &crate::md::Sentence,
        with_code: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("STL003") {
            for run in with_code {
                let mut seen = BTreeSet::new();
                for context in words(self.config, sentence.language, Words::Commit) {
                    for occurrence in context.occurrences(&run.text) {
                        let tail = &run.text[occurrence.end..];
                        // Require a separator: `commitdeadbee` is one identifier, not a context and hash.
                        if context
                            .text
                            .ends_with(|ch: char| ch.is_ascii_alphanumeric())
                            && !tail
                                .starts_with(|ch: char| ch.is_whitespace() || ":=#".contains(ch))
                        {
                            continue;
                        }
                        if let Some(hash) = HASH.captures(tail).and_then(|caps| caps.name("hash")) {
                            let range = Span::new(
                                occurrence.end + hash.start(),
                                occurrence.end + hash.end(),
                            );
                            if let Some(span) = run.span(range).filter(|span| seen.insert(*span)) {
                                diagnostics.push(emit(
                                    self.document,
                                    self.filename,
                                    "STL003",
                                    span,
                                    "A commit or build identifier is embedded in a long-lived document.",
                                    "Point to the command or release record that provides the identifier.",
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_root_pointers(
        &self,
        sentence: &crate::md::Sentence,
        prose: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("PTR001")
            && marker(
                prose,
                &words(self.config, sentence.language, Words::Pointer),
            )
            .is_some()
        {
            for link in self.document.links.iter().filter(|link| {
                !link.image
                    && sentence.span.start < link.span.end
                    && link.span.start < sentence.span.end
            }) {
                if repository_root(&link.destination, self.path, self.workspace_root) {
                    let span = if link.reference.is_some() {
                        link.span
                    } else {
                        link.destination_span
                    };
                    diagnostics.push(emit(
                        self.document,
                        self.filename,
                        "PTR001",
                        span,
                        "This pointer links to a repository root.",
                        "Link to the file, symbol, or heading that answers the reader's question.",
                    ));
                }
            }
        }
    }

    fn check_source_pointers(
        &self,
        sentence: &crate::md::Sentence,
        prose: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("PTR003")
            && let Some(span) = marker(prose, &words(self.config, sentence.language, Words::Source))
            && !sentence.fragments.iter().any(specific_target)
        {
            diagnostics.push(emit(
                self.document,
                self.filename,
                "PTR003",
                span,
                "This source pointer names no file or symbol.",
                "Name the source file and a searchable symbol, or link directly to the relevant definition.",
            ));
        }
    }

    fn check_rationale_headings(
        &self,
        sentence: &crate::md::Sentence,
        prose: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("RAT002")
            && self.document.blocks[sentence.block].kind == BlockKind::Heading
        {
            let span = prose.iter().find_map(|run| {
                let start = run.text.len() - run.text.trim_start().len();
                words(self.config, sentence.language, Words::Rationale)
                    .iter()
                    .find_map(|phrase| {
                        phrase
                            .occurrences(&run.text)
                            .into_iter()
                            .find(|range| range.start == start)
                            .and_then(|range| run.span(range))
                    })
                    .or_else(|| {
                        (sentence.language == Language::Ja)
                            .then(|| JAPANESE_CHOICE.find(&run.text))
                            .flatten()
                            .and_then(|m| run.span(Span::new(m.start(), m.end())))
                    })
            });
            if let Some(span) = span {
                diagnostics.push(emit(
                    self.document,
                    self.filename,
                    "RAT002",
                    span,
                    "This heading introduces a design-choice rationale in a how-to or reference page.",
                    "Move the decision rationale to an ADR and keep the procedure or contract here.",
                ));
            }
        }
    }

    fn check_conversation(
        &self,
        sentence: &crate::md::Sentence,
        prose: &[Run<'_>],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.enabled.contains("VOX001")
            && let Some(span) = marker(
                prose,
                &words(self.config, sentence.language, Words::Conversation),
            )
        {
            diagnostics.push(emit(
                self.document,
                self.filename,
                "VOX001",
                span,
                "This phrase refers to the requester of an earlier conversation.",
                "State the document's instructions or facts directly for its readers.",
            ));
        }
    }
}

fn specific_target(fragment: &Fragment) -> bool {
    let text = fragment.text.trim();
    if fragment.kind == FragmentKind::LinkDestination {
        if text.starts_with('#') {
            return text.len() > 1;
        }
        let path = text.split(['?', '#']).next().unwrap_or(text);
        return FILE.is_match(path)
            || path.rsplit('/').next().is_some_and(|name| {
                matches!(name, "Dockerfile" | "Makefile" | "LICENSE" | "Cargo.lock")
            })
            || text
                .split_once('#')
                .is_some_and(|(_, fragment)| !fragment.is_empty());
    }
    let visible = URL.replace_all(text, " ");
    FILE.is_match(&visible)
        || SYMBOL.is_match(&visible)
        || (fragment.kind == FragmentKind::InlineCode
            && (matches!(text, "Dockerfile" | "Makefile" | "LICENSE")
                || (text.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                    && text
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))))
}

fn repository_root(destination: &str, path: &Path, root: &Path) -> bool {
    if destination.contains(['#', '?', '{', '}', '$']) {
        return false;
    }
    if let Some((scheme, remainder)) = destination.split_once("://") {
        if !["http", "https"].contains(&scheme.to_ascii_lowercase().as_str()) {
            return false;
        }
        let Some((host, pathname)) = remainder.split_once('/') else {
            return false;
        };
        if !["github.com", "gitlab.com", "codeberg.org", "bitbucket.org"]
            .contains(&host.to_ascii_lowercase().as_str())
        {
            return false;
        }
        let parts: Vec<_> = pathname.trim_matches('/').split('/').collect();
        let reserved: &[&str] = match host.to_ascii_lowercase().as_str() {
            "github.com" => &[
                "about",
                "apps",
                "collections",
                "customer-stories",
                "enterprise",
                "explore",
                "features",
                "login",
                "marketplace",
                "new",
                "notifications",
                "orgs",
                "pricing",
                "readme",
                "search",
                "security",
                "settings",
                "signup",
                "site",
                "sponsors",
                "topics",
                "users",
            ],
            "gitlab.com" => &["-", "admin", "dashboard", "explore", "help", "users"],
            "codeberg.org" => &[
                "admin",
                "explore",
                "issues",
                "notifications",
                "org",
                "pulls",
                "repo",
                "user",
            ],
            "bitbucket.org" => &[
                "account",
                "dashboard",
                "plans",
                "product",
                "site",
                "support",
            ],
            _ => &[],
        };
        return parts.len() == 2
            && parts.iter().all(|part| !part.is_empty())
            && !reserved.contains(&parts[0].to_ascii_lowercase().as_str());
    }
    if destination.starts_with("//") || destination.contains(':') || destination.is_empty() {
        return false;
    }
    local_link_target(root, path, destination).is_ok_and(|link| link.path == normalize(root))
}
