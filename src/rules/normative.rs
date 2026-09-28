use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::config::{Config, Lexicon};
use crate::diagnostics::{Diagnostic, Span};
use crate::md::prose::{Run, marker, occurrences, runs};
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

fn defaults(words: Words, language: Language) -> &'static [&'static str] {
    match (words, language) {
        (Words::Stale, Language::En) => &["currently", "latest", "at present"],
        (Words::Stale, Language::Zh) => &["目前", "当前", "最新", "现在"],
        (Words::Stale, Language::Ja) => &["現在", "最新", "現時点"],
        (Words::Constraint, Language::En) => &[
            "or later", "or newer", "at least", "at most", "minimum", "maximum", "requires",
            "required",
        ],
        (Words::Constraint, Language::Zh) => {
            &["以上", "以下", "至少", "最多", "要求", "不低于", "不高于"]
        }
        (Words::Constraint, Language::Ja) => {
            &["以上", "以下", "以降", "少なくとも", "必要", "要件"]
        }
        (Words::Commit, Language::En) => &["commit", "sha", "build"],
        (Words::Commit, Language::Zh) => &["commit", "sha", "build", "提交", "构建"],
        (Words::Commit, Language::Ja) => &["commit", "sha", "build", "コミット", "ビルド"],
        (Words::Pointer, Language::En) => &["see", "defined in", "refer to"],
        (Words::Pointer, Language::Zh) => &["见", "参见", "参考", "定义在"],
        (Words::Pointer, Language::Ja) => &["参照", "定義", "をご覧"],
        (Words::Source, Language::En) => &[
            "see the source",
            "see source",
            "refer to the source",
            "see the code",
        ],
        (Words::Source, Language::Zh) => {
            &["见源码", "见源代码", "参见源码", "参见源代码", "参考源码"]
        }
        (Words::Source, Language::Ja) => &["ソースを参照", "ソースコードを参照", "コードを参照"],
        (Words::Rationale, Language::En) => {
            &["why we chose", "why we choose", "why we use", "why not use"]
        }
        (Words::Rationale, Language::Zh) => &[
            "为什么选择",
            "为何选择",
            "为什么不用",
            "为何不用",
            "为什么采用",
        ],
        (Words::Rationale, Language::Ja) => &["採用した理由", "選択した理由"],
        (Words::Conversation, Language::En) => &[
            "as you requested",
            "as requested by you",
            "here's the updated",
            "here’s the updated",
            "here is the updated",
            "i hope this helps",
            "hope this helps",
        ],
        (Words::Conversation, Language::Zh) => &[
            "根据你的要求",
            "根据您的要求",
            "按你的要求",
            "按照你的要求",
            "希望对你有帮助",
            "希望对您有帮助",
        ],
        (Words::Conversation, Language::Ja) => &[
            "ご要望に応じて",
            "ご依頼のとおり",
            "ご依頼どおり",
            "お役に立てれば幸い",
        ],
    }
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

pub(crate) fn language_key(language: Language) -> &'static str {
    match language {
        Language::En => "en",
        Language::Zh => "zh",
        Language::Ja => "ja",
    }
}

fn words(config: &Config, language: Language, kind: Words) -> Vec<&str> {
    let mut values = defaults(kind, language).to_vec();
    if let Some(lexicon) = config.settings.lint.lexicon.get(language_key(language)) {
        values.extend(extensions(kind, lexicon).iter().map(String::as_str));
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
    let mut diagnostics = Vec::new();
    for sentence in &document.sentences {
        if !config
            .settings
            .lint
            .languages
            .iter()
            .any(|lang| lang == language_key(sentence.language))
        {
            continue;
        }
        let prose = runs(sentence, false);
        let with_code = runs(sentence, true);
        if enabled.contains("STL001") {
            let stale = marker(&prose, &words(config, sentence.language, Words::Stale));
            let constraint = marker(
                &with_code,
                &words(config, sentence.language, Words::Constraint),
            );
            if stale.is_some()
                && constraint.is_none()
                && !with_code.iter().any(|run| BOUND.is_match(&run.text))
                && let Some(span) = with_code.iter().find_map(volatile_value)
            {
                diagnostics.push(emit(document, filename, "STL001", span,
                        "This sentence pairs a current-state marker with a value that can change.",
                        "Link to the source that owns the value, or state a lasting requirement with its constraint."));
            }
        }
        if enabled.contains("STL003") {
            for run in &with_code {
                let mut seen = BTreeSet::new();
                for context in words(config, sentence.language, Words::Commit) {
                    for occurrence in occurrences(&run.text, context) {
                        let tail = &run.text[occurrence.end..];
                        // Require a separator: `commitdeadbee` is one identifier, not a context and hash.
                        if context.ends_with(|ch: char| ch.is_ascii_alphanumeric())
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
                                diagnostics.push(emit(document, filename, "STL003", span,
                                    "A commit or build identifier is embedded in a long-lived document.",
                                    "Point to the command or release record that provides the identifier."));
                            }
                        }
                    }
                }
            }
        }
        if enabled.contains("PTR001")
            && marker(&prose, &words(config, sentence.language, Words::Pointer)).is_some()
        {
            for link in document.links.iter().filter(|link| {
                !link.image
                    && sentence.span.start < link.span.end
                    && link.span.start < sentence.span.end
            }) {
                if repository_root(&link.destination, path, workspace_root) {
                    let span = if link.reference.is_some() {
                        link.span
                    } else {
                        link.destination_span
                    };
                    diagnostics.push(emit(
                        document,
                        filename,
                        "PTR001",
                        span,
                        "This pointer links to a repository root.",
                        "Link to the file, symbol, or heading that answers the reader's question.",
                    ));
                }
            }
        }
        if enabled.contains("PTR003")
            && let Some(span) = marker(&prose, &words(config, sentence.language, Words::Source))
            && !sentence.fragments.iter().any(specific_target)
        {
            diagnostics.push(emit(document, filename, "PTR003", span,
                        "This source pointer names no file or symbol.",
                        "Name the source file and a searchable symbol, or link directly to the relevant definition."));
        }
        if enabled.contains("RAT002") && document.blocks[sentence.block].kind == BlockKind::Heading
        {
            let span = prose.iter().find_map(|run| {
                let start = run.text.len() - run.text.trim_start().len();
                words(config, sentence.language, Words::Rationale)
                    .iter()
                    .find_map(|phrase| {
                        occurrences(&run.text, phrase)
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
                diagnostics.push(emit(document, filename, "RAT002", span,
                    "This heading introduces a design-choice rationale in a how-to or reference page.",
                    "Move the decision rationale to an ADR and keep the procedure or contract here."));
            }
        }
        if enabled.contains("VOX001")
            && let Some(span) = marker(
                &prose,
                &words(config, sentence.language, Words::Conversation),
            )
        {
            diagnostics.push(emit(
                document,
                filename,
                "VOX001",
                span,
                "This phrase addresses the requester of an earlier conversation.",
                "State the document's instructions or facts directly for its readers.",
            ));
        }
    }
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert((diagnostic.code.clone(), diagnostic.byte_range)));
    diagnostics
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
