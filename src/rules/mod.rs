//! Single-document rules, rule documentation, and suppression evaluation.

pub mod cross_file;
pub mod fixes;
pub mod heuristic;
mod kind;
mod links;
pub mod normative;
pub mod suppression;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::Path;

use crate::config::{CliOverrides, Config, ConfigError};
use crate::diagnostics::{Diagnostic, Span, sorted_diagnostics};
use crate::index::IndexedFile;
use crate::md::{Document, FragmentKind};
use crate::workspace::FilePolicy;
use serde::Serialize;

pub use kind::{Kind, KindOutcome, KindResolution, resolve_kind};
pub use links::{LocalWorkspaceFiles, WorkspaceFiles};
pub use suppression::SuppressionRecord;

/// One document and the policy that its single-document rules run under.
pub struct CheckContext<'a> {
    document: &'a Document,
    path: &'a Path,
    workspace_root: &'a Path,
    config: &'a Config,
    policy: Cow<'a, FilePolicy>,
}

impl<'a> CheckContext<'a> {
    /// Resolve the policy of a document that no workspace index holds.
    pub fn new(
        document: &'a Document,
        filename: &str,
        path: &'a Path,
        workspace_root: &'a Path,
        config: &'a Config,
        overrides: &CliOverrides,
    ) -> Result<Self, ConfigError> {
        let policy = FilePolicy::resolve(filename.to_owned(), path, document, config, overrides)?;
        Ok(Self {
            document,
            path,
            workspace_root,
            config,
            policy: Cow::Owned(policy),
        })
    }

    /// Borrow the policy resolved when the workspace was loaded.
    pub fn indexed(file: &'a IndexedFile, workspace_root: &'a Path) -> Self {
        Self {
            document: file.document(),
            path: file.path(),
            workspace_root,
            config: file.config(),
            policy: Cow::Borrowed(file.policy()),
        }
    }

    pub fn filename(&self) -> &str {
        &self.policy.filename
    }
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

/// Run the policy's single-document rules. Index rules count as enabled only
/// once [`RawCheckResult::add_cross_file`] merges their results.
pub fn check(context: &CheckContext<'_>, files: &dyn WorkspaceFiles) -> RawCheckResult {
    let kind = &context.policy.kind;
    let enabled = policy_rules(&context.policy, false);
    let mut diagnostics: Vec<_> = kind
        .diagnostic(context.document, context.filename())
        .filter(|diagnostic| enabled.contains(&diagnostic.code))
        .into_iter()
        .collect();
    if enabled
        .iter()
        .any(|code| rule(code).is_some_and(|rule| rule.phase == RulePhase::Normative))
    {
        diagnostics.extend(normative::check(
            context.document,
            context.filename(),
            context.path,
            context.workspace_root,
            context.config,
            &enabled,
        ));
    }
    let mut incomplete = BTreeSet::new();
    if enabled
        .iter()
        .any(|code| rule(code).is_some_and(|rule| rule.phase == RulePhase::Heuristic))
    {
        let heuristic = heuristic::check(
            context.document,
            context.filename(),
            context.config,
            &enabled,
        );
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
    RawCheckResult {
        kind: kind.resolution(),
        enabled_rules: enabled,
        diagnostics,
        incomplete_rules: incomplete,
        errors,
    }
}

/// The policy's enabled rules that do, or do not, require the workspace index.
fn policy_rules(policy: &FilePolicy, requires_index: bool) -> BTreeSet<String> {
    policy
        .enabled_rules
        .iter()
        .filter(|code| rule(code).is_some_and(|rule| rule.requires_index == requires_index))
        .cloned()
        .collect()
}

impl RawCheckResult {
    /// Describe a document whose single-document rules did not run.
    pub fn unchecked(policy: &FilePolicy) -> Self {
        Self {
            kind: policy.kind.resolution(),
            enabled_rules: policy_rules(policy, false),
            diagnostics: Vec::new(),
            incomplete_rules: single_file_rules()
                .map(|rule| rule.code.to_owned())
                .collect(),
            errors: Vec::new(),
        }
    }

    /// Merge the workspace rule results for this document and enable its index rules.
    pub fn add_cross_file(
        &mut self,
        policy: &FilePolicy,
        diagnostics: Vec<Diagnostic>,
        incomplete: BTreeSet<String>,
    ) {
        self.enabled_rules.extend(policy_rules(policy, true));
        self.diagnostics.extend(diagnostics);
        self.incomplete_rules.extend(incomplete);
    }

    pub fn finish(self, document: &Document, filename: &str) -> CheckResult {
        let RawCheckResult {
            kind,
            enabled_rules: enabled,
            diagnostics,
            incomplete_rules: incomplete,
            errors,
        } = self;
        let mut result = if kind.value == Some(Kind::Generated) {
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

    pub fn applies_to_kind(&self, kind: Option<Kind>) -> bool {
        if kind == Some(Kind::Generated) {
            return false;
        }
        let Some(kind) = kind else {
            return self.kinds == KindScope::Any;
        };
        match self.kinds {
            KindScope::Any | KindScope::Declared => true,
            KindScope::LongLived => {
                [Kind::Readme, Kind::Howto, Kind::Reference, Kind::Runbook].contains(&kind)
            }
            KindScope::ExceptChangelog => kind != Kind::Changelog,
            KindScope::HowtoOrReference => [Kind::Howto, Kind::Reference].contains(&kind),
            KindScope::HowtoOrRunbook => [Kind::Howto, Kind::Runbook].contains(&kind),
            KindScope::Procedural => [Kind::Howto, Kind::Reference, Kind::Runbook].contains(&kind),
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
