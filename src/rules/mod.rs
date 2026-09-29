//! Single-document rules, rule documentation, and suppression evaluation.

pub mod cross_file;
pub mod fixes;
pub mod heuristic;
mod links;
pub mod normative;
pub mod suppression;

use std::collections::BTreeSet;
use std::path::Path;

use crate::config::{CliOverrides, Config, ConfigError};
use crate::diagnostics::{Diagnostic, Span, sorted_diagnostics};
use crate::md::{Document, FragmentKind};
use serde::Serialize;

pub use links::{LocalWorkspaceFiles, PathStatus, WorkspaceFiles};
pub use suppression::SuppressionRecord;

#[derive(Clone, Debug, Serialize)]
pub struct KindResolution {
    pub value: Option<String>,
    pub source: &'static str,
    pub problem: Option<String>,
}

pub fn resolve_kind(document: &Document, mapped: Option<&str>) -> KindResolution {
    if let Some(frontmatter) = &document.frontmatter {
        if !frontmatter.errors.is_empty() {
            return KindResolution {
                value: None,
                source: "frontmatter",
                problem: Some(
                    "Frontmatter is invalid; inspect document.frontmatter.errors.".into(),
                ),
            };
        }
        if let Some(kind) = &frontmatter.kind {
            return if kind != "generated" && crate::config::KINDS.contains(&kind.as_str()) {
                KindResolution {
                    value: Some(kind.clone()),
                    source: "frontmatter",
                    problem: None,
                }
            } else {
                KindResolution {
                    value: None,
                    source: "frontmatter",
                    problem: Some(if kind == "generated" {
                        "The generated kind can only be assigned in configuration.".into()
                    } else {
                        format!(
                            "Unknown kind {kind:?}; use one of the lowercase kinds {}.",
                            declarable_kinds()
                        )
                    }),
                }
            };
        }
    }
    KindResolution {
        value: mapped.map(str::to_owned),
        source: if mapped.is_some() {
            "configuration"
        } else {
            "unknown"
        },
        problem: mapped.is_none().then(|| {
            "Declare kind in frontmatter or add a matching [[kinds]] configuration entry.".into()
        }),
    }
}

/// Kinds that frontmatter can declare, listed for diagnostic text.
fn declarable_kinds() -> String {
    let kinds: Vec<_> = crate::config::KINDS
        .into_iter()
        .filter(|kind| *kind != "generated")
        .collect();
    match kinds.split_last() {
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}

pub struct CheckContext<'a> {
    pub document: &'a Document,
    pub filename: &'a str,
    pub path: &'a Path,
    pub workspace_root: &'a Path,
    pub config: &'a Config,
    pub overrides: &'a CliOverrides,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub kind: KindResolution,
    pub enabled_rules: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub suppressions: Vec<SuppressionRecord>,
    pub errors: Vec<String>,
}

pub struct RawCheckResult {
    pub kind: KindResolution,
    pub enabled_rules: BTreeSet<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub incomplete_rules: BTreeSet<String>,
    pub errors: Vec<String>,
}

pub fn check_raw(context: &CheckContext<'_>) -> Result<RawCheckResult, ConfigError> {
    check_raw_with_files(context, &LocalWorkspaceFiles::default())
}

pub fn check(context: &CheckContext<'_>) -> Result<CheckResult, ConfigError> {
    check_with_files(context, &LocalWorkspaceFiles::default())
}

/// Use a frozen file inventory while preserving the production link resolver.
pub fn check_with_files(
    context: &CheckContext<'_>,
    files: &dyn WorkspaceFiles,
) -> Result<CheckResult, ConfigError> {
    Ok(finish_check(
        context.document,
        context.filename,
        check_raw_with_files(context, files)?,
    ))
}

pub fn check_raw_with_files(
    context: &CheckContext<'_>,
    files: &dyn WorkspaceFiles,
) -> Result<RawCheckResult, ConfigError> {
    let kind = resolve_kind(context.document, context.config.kind_for(context.path));
    let enabled: BTreeSet<String> = context
        .config
        .enabled_rules(context.path, kind.value.as_deref(), context.overrides)?
        .into_iter()
        .filter(|code| rule(code).is_some_and(|rule| !rule.requires_index))
        .map(str::to_owned)
        .collect();
    let mut diagnostics = kind_diagnostics(context, &enabled);
    if enabled
        .iter()
        .any(|code| rule(code).is_some_and(|rule| rule.phase == RulePhase::Normative))
    {
        let written_root = files.written_root(context.workspace_root, context.path);
        diagnostics.extend(normative::check(
            context.document,
            context.filename,
            context.path,
            written_root.as_ref(),
            context.config,
            &enabled,
        ));
    }
    let mut incomplete = BTreeSet::new();
    if enabled
        .iter()
        .any(|code| rule(code).is_some_and(|rule| rule.phase == RulePhase::Heuristic))
    {
        let heuristic =
            heuristic::check(context.document, context.filename, context.config, &enabled);
        diagnostics.extend(heuristic.diagnostics);
        incomplete.extend(heuristic.incomplete_rules);
    }
    // One value gets one explanation: identifier and version rules say more about
    // it than STL001's generic current-state diagnosis.
    let specific: BTreeSet<Span> = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.code.as_str(), "STL003" | "STL004"))
        .map(|diagnostic| diagnostic.byte_range)
        .collect();
    diagnostics.retain(|diagnostic| {
        diagnostic.code != "STL001" || !specific.contains(&diagnostic.byte_range)
    });
    let mut errors = Vec::new();
    if enabled.contains("LNK001") {
        let links = links::check(context, files);
        diagnostics.extend(links.diagnostics);
        errors = links.errors;
        if links.incomplete {
            incomplete.insert("LNK001".to_owned());
        }
    }
    Ok(RawCheckResult {
        kind,
        enabled_rules: enabled,
        diagnostics,
        incomplete_rules: incomplete,
        errors,
    })
}

pub fn finish_check(document: &Document, filename: &str, raw: RawCheckResult) -> CheckResult {
    let RawCheckResult {
        kind,
        enabled_rules: enabled,
        diagnostics,
        incomplete_rules: incomplete,
        errors,
    } = raw;
    let mut result = if kind.value.as_deref() == Some("generated") {
        suppression::SuppressionResult {
            diagnostics: Vec::new(),
            suppressions: Vec::new(),
        }
    } else {
        suppression::apply(document, filename, diagnostics, &enabled, &incomplete)
    };
    fixes::attach_fixes(document, &result.suppressions, &mut result.diagnostics);
    let diagnostics = sorted_diagnostics(&result.diagnostics);
    CheckResult {
        kind,
        enabled_rules: enabled.into_iter().collect(),
        diagnostics,
        suppressions: result.suppressions,
        errors,
    }
}

fn kind_diagnostics(context: &CheckContext<'_>, enabled: &BTreeSet<String>) -> Vec<Diagnostic> {
    let document = context.document;
    let mut code = "KND001";
    let mut span = Span::new(0, 0);
    let mut message =
        "Document kind is not declared and no kind mapping matches this file.".to_owned();
    let mut suggestion = format!(
        "Declare kind as {} in YAML frontmatter, or add a matching [[kinds]] configuration entry.",
        declarable_kinds()
    );
    if let Some(frontmatter) = &document.frontmatter {
        span = frontmatter.span;
        if let Some(error) = frontmatter.errors.first() {
            span = error.span;
            message = format!(
                "Document kind cannot be resolved because the frontmatter is invalid: {}.",
                error.message.trim_end_matches('.')
            );
            suggestion = "Correct the YAML frontmatter so its kind declaration can be read.".into();
        } else if let Some(kind) = &frontmatter.kind {
            if kind != "generated" && crate::config::KINDS.contains(&kind.as_str()) {
                return Vec::new();
            }
            code = "KND002";
            if kind == "generated" {
                message = "The generated kind is declared in frontmatter; it can only be assigned in configuration.".into();
                suggestion = "Remove this declaration and assign generated with a [[kinds]] path mapping if a tool generates this file.".into();
            } else {
                message = format!("Unknown document kind {kind:?}.");
                suggestion = format!(
                    "Use one of the lowercase kinds {} in frontmatter.",
                    declarable_kinds()
                );
            }
        } else if context.config.kind_for(context.path).is_some() {
            return Vec::new();
        }
    } else if context.config.kind_for(context.path).is_some() {
        return Vec::new();
    }
    if enabled.contains(code) {
        vec![Diagnostic::new(
            context.filename,
            &document.source,
            code,
            span,
            message,
            suggestion,
        )]
    } else {
        Vec::new()
    }
}

#[derive(Debug, Serialize)]
pub struct Rule {
    pub code: &'static str,
    pub name: &'static str,
    pub basis: &'static str,
    pub reads: &'static [FragmentKind],
    pub requires_filesystem: bool,
    pub requires_index: bool,
    pub documentation: &'static str,
    #[serde(skip)]
    phase: RulePhase,
    #[serde(skip)]
    stable: bool,
    #[serde(skip)]
    kinds: KindScope,
}

impl Rule {
    pub fn status(&self) -> &'static str {
        if self.stable { "stable" } else { "preview" }
    }

    pub fn is_stable(&self) -> bool {
        self.stable
    }

    pub fn applies_to_kind(&self, kind: Option<&str>) -> bool {
        if kind == Some("generated") {
            return false;
        }
        let Some(kind) = kind.filter(|kind| crate::config::KINDS.contains(kind)) else {
            return self.kinds == KindScope::Any;
        };
        match self.kinds {
            KindScope::Any | KindScope::Declared => true,
            KindScope::LongLived => ["readme", "howto", "reference", "runbook"].contains(&kind),
            KindScope::ExceptChangelog => kind != "changelog",
            KindScope::HowtoOrReference => ["howto", "reference"].contains(&kind),
            KindScope::HowtoOrRunbook => ["howto", "runbook"].contains(&kind),
            KindScope::Procedural => ["howto", "reference", "runbook"].contains(&kind),
        }
    }

    /// This rule's page at the tag of this seiso version.
    pub fn url(&self) -> String {
        repository_url(&format!("docs/rules/{}.md", self.code))
    }

    /// The embedded page with its relative links pointing at this version's
    /// tag, so they still open outside a seiso checkout.
    pub fn standalone_documentation(&self) -> String {
        let source = self.documentation;
        let Ok(document) = crate::md::parse(source) else {
            return source.to_owned();
        };
        let mut spans: Vec<_> = document
            .links
            .iter()
            // Only a destination written without escapes can be replaced at its span.
            .filter(|link| {
                let span = link.destination_span;
                source.get(span.start..span.end) == Some(link.destination.as_str())
                    && !link.destination.starts_with(['#', '/'])
                    && !crate::paths::has_scheme(&link.destination)
            })
            .map(|link| link.destination_span)
            .collect();
        spans.sort_by_key(|span| span.start);
        // Reference links share their definition's span.
        spans.dedup();
        let mut output = source.to_owned();
        for span in spans.into_iter().rev() {
            let destination = &source[span.start..span.end];
            let (path, suffix) =
                destination.split_at(destination.find(['#', '?']).unwrap_or(destination.len()));
            let mut segments = vec!["docs", "rules"];
            for segment in path.split('/') {
                match segment {
                    "" | "." => {}
                    ".." => {
                        segments.pop();
                    }
                    segment => segments.push(segment),
                }
            }
            output.replace_range(
                span.start..span.end,
                &(repository_url(&segments.join("/")) + suffix),
            );
        }
        output
    }
}

/// A repository file at the tag of this seiso version.
fn repository_url(path: &str) -> String {
    format!(
        "https://github.com/scarletkc/seiso/blob/v{}/{path}",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Debug, PartialEq, Eq)]
enum RulePhase {
    Kind,
    Normative,
    Links,
    Suppression,
    CrossFile,
    Heuristic,
}

#[derive(Debug, PartialEq, Eq)]
enum KindScope {
    Any,
    Declared,
    LongLived,
    ExceptChangelog,
    HowtoOrReference,
    HowtoOrRunbook,
    Procedural,
}

macro_rules! rule {
    ($code:literal, $name:literal, $basis:literal, $reads:expr, $fs:expr, $phase:ident, $stable:expr, $kinds:ident) => {
        Rule {
            code: $code,
            name: $name,
            basis: $basis,
            reads: $reads,
            requires_filesystem: $fs,
            requires_index: matches!(RulePhase::$phase, RulePhase::CrossFile),
            documentation: include_str!(concat!("../../docs/rules/", $code, ".md")),
            phase: RulePhase::$phase,
            stable: $stable,
            kinds: KindScope::$kinds,
        }
    };
}

use FragmentKind::{InlineCode, LinkDestination, LinkText, Text};
static RULES: &[Rule] = &[
    rule!(
        "STL002",
        "deployment-state-assertion",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        LongLived
    ),
    rule!(
        "STL004",
        "unconstrained-version",
        "heuristic",
        &[Text, LinkText, InlineCode],
        false,
        Heuristic,
        false,
        LongLived
    ),
    rule!(
        "RAT001",
        "rationale-before-procedure",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        HowtoOrReference
    ),
    rule!(
        "ORD001",
        "long-preamble",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        HowtoOrRunbook
    ),
    rule!(
        "ORD002",
        "exceptions-before-procedure",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        HowtoOrRunbook
    ),
    rule!(
        "MIX001",
        "conflicting-section-role",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        Procedural
    ),
    rule!(
        "VOX002",
        "excluded-scope-heading",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        LongLived
    ),
    rule!(
        "VOX003",
        "production-narration",
        "heuristic",
        &[Text, LinkText],
        false,
        Heuristic,
        false,
        LongLived
    ),
    rule!(
        "EVD001",
        "unsupported-evaluation",
        "heuristic",
        &[Text, LinkText, InlineCode, LinkDestination],
        false,
        Heuristic,
        false,
        Declared
    ),
    rule!(
        "KND001",
        "missing-document-kind",
        "consistency",
        &[],
        false,
        Kind,
        true,
        Any
    ),
    rule!(
        "KND002",
        "invalid-document-kind",
        "consistency",
        &[],
        false,
        Kind,
        true,
        Any
    ),
    rule!(
        "STL001",
        "current-value-snapshot",
        "convention",
        &[Text, LinkText, InlineCode],
        false,
        Normative,
        false,
        LongLived
    ),
    rule!(
        "STL003",
        "commit-snapshot",
        "convention",
        &[Text, LinkText, InlineCode],
        false,
        Normative,
        false,
        LongLived
    ),
    rule!(
        "PTR001",
        "repository-root-pointer",
        "convention",
        &[Text, LinkText, LinkDestination],
        false,
        Normative,
        false,
        ExceptChangelog
    ),
    rule!(
        "PTR003",
        "unspecified-source-pointer",
        "convention",
        &[Text, LinkText, InlineCode, LinkDestination],
        false,
        Normative,
        false,
        Declared
    ),
    rule!(
        "LNK001",
        "missing-local-link-target",
        "consistency",
        &[LinkDestination],
        true,
        Links,
        true,
        Any
    ),
    rule!(
        "RAT002",
        "design-choice-heading",
        "convention",
        &[Text, LinkText],
        false,
        Normative,
        false,
        HowtoOrReference
    ),
    rule!(
        "VOX001",
        "requester-conversation",
        "convention",
        &[Text, LinkText],
        false,
        Normative,
        false,
        LongLived
    ),
    rule!(
        "SUP001",
        "invalid-suppression",
        "consistency",
        &[],
        false,
        Suppression,
        true,
        Any
    ),
    rule!(
        "SUP002",
        "unused-suppression",
        "consistency",
        &[],
        false,
        Suppression,
        true,
        Any
    ),
    rule!(
        "PTR002",
        "directory-pointer",
        "heuristic",
        &[LinkDestination],
        true,
        CrossFile,
        false,
        ExceptChangelog
    ),
    rule!(
        "LNK002",
        "missing-local-anchor",
        "consistency",
        &[LinkDestination],
        true,
        CrossFile,
        false,
        Any
    ),
    rule!(
        "DUP001",
        "repeated-definitions",
        "heuristic",
        &[InlineCode],
        false,
        CrossFile,
        false,
        Declared
    ),
    rule!(
        "DUP002",
        "repeated-content-pointer",
        "heuristic",
        &[InlineCode, LinkDestination],
        false,
        CrossFile,
        false,
        Declared
    ),
    rule!(
        "DUP003",
        "similar-paragraphs",
        "heuristic",
        &[Text, LinkText, InlineCode],
        false,
        CrossFile,
        false,
        Declared
    ),
    rule!(
        "OWN001",
        "plan-definition-overlap",
        "heuristic",
        &[InlineCode],
        false,
        CrossFile,
        false,
        Declared
    ),
    rule!(
        "OWN002",
        "ambiguous-owner",
        "heuristic",
        &[InlineCode, Text, LinkText],
        false,
        CrossFile,
        false,
        Declared
    ),
];

pub fn rules() -> &'static [Rule] {
    RULES
}

pub fn rule(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.code == code)
}

pub fn rule_codes() -> impl Iterator<Item = &'static str> {
    RULES.iter().map(|rule| rule.code)
}

pub fn single_file_rules() -> impl Iterator<Item = &'static Rule> {
    RULES.iter().filter(|rule| !rule.requires_index)
}

pub fn cross_file_rules() -> impl Iterator<Item = &'static Rule> {
    RULES.iter().filter(|rule| rule.requires_index)
}
