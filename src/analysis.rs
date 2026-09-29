//! Run enabled rules against a loaded snapshot and select the reported diagnostics.

use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, sorted_diagnostics};
use crate::rules::{self, CheckContext, Kind, RawCheckResult};
use crate::workspace::{InputError, Snapshot};

pub struct Analysis {
    pub snapshot: Snapshot,
    pub diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    pub fn exit_code(&self, exit_zero: bool) -> u8 {
        if !self.snapshot.errors.is_empty() {
            2
        } else if !exit_zero && !self.diagnostics.is_empty() {
            1
        } else {
            0
        }
    }

    pub fn has_fixes(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| {
            diagnostic.fix.as_ref().is_some_and(|fix| {
                fix.applicability == crate::diagnostics::Applicability::Safe
                    && !fix.edits.is_empty()
            })
        })
    }

    pub fn same_inputs_and_diagnostics(&self, other: &Self) -> bool {
        self.diagnostics == other.diagnostics
            && self.snapshot.index.files().len() == other.snapshot.index.files().len()
            && self
                .snapshot
                .index
                .files()
                .iter()
                .zip(other.snapshot.index.files())
                .all(|(a, b)| {
                    a.policy.filename == b.policy.filename && a.document.source == b.document.source
                })
    }
}

/// Inspect declarations without claiming that their rules have executed.
pub fn inspect_policy(snapshot: &mut Snapshot) {
    let records = snapshot
        .index
        .files()
        .iter()
        .filter(|file| file.policy.kind_value() != Some(Kind::Generated))
        .map(|file| {
            let enabled = file.policy.enabled_rules.iter().cloned().collect();
            (
                file.policy.filename.clone(),
                rules::suppression::inspect(&file.document, &enabled),
            )
        })
        .collect();
    snapshot.index.set_suppressions(records);
}

pub fn check(mut snapshot: Snapshot) -> Result<Analysis, String> {
    let index = &snapshot.index;
    let mut cross = if index.files().iter().any(|file| {
        file.policy
            .enabled_rules
            .iter()
            .any(|code| rules::rule(code).is_some_and(|rule| rule.requires_index))
    }) {
        rules::cross_file::check(index)
    } else {
        rules::cross_file::CrossReport::default()
    };
    snapshot.errors.extend(
        cross
            .errors
            .into_iter()
            .map(|(filename, message)| InputError { filename, message }),
    );
    let mut cross_by_file = BTreeMap::<String, Vec<Diagnostic>>::new();
    for diagnostic in cross.diagnostics {
        cross_by_file
            .entry(diagnostic.filename.clone())
            .or_default()
            .push(diagnostic);
    }
    let mut diagnostics = Vec::new();
    let mut suppressions = BTreeMap::new();
    for file in index.files() {
        let selected = snapshot.selected.contains(&file.policy.filename);
        if !selected && !cross_by_file.contains_key(&file.policy.filename) {
            continue;
        }
        let mut raw = if selected {
            rules::check(
                &CheckContext {
                    document: &file.document,
                    filename: &file.policy.filename,
                    path: &file.path,
                    workspace_root: &index.root,
                    config: &file.config,
                    policy: std::borrow::Cow::Borrowed(&file.policy),
                },
                index,
            )
        } else {
            RawCheckResult {
                kind: file
                    .policy
                    .kind
                    .as_ref()
                    .expect("included document policy")
                    .resolution(),
                enabled_rules: file.policy.enabled_rules.iter().cloned().collect(),
                diagnostics: Vec::new(),
                errors: Vec::new(),
                incomplete_rules: rules::single_file_rules()
                    .map(|rule| rule.code.to_owned())
                    .collect(),
            }
        };
        raw.diagnostics.extend(
            cross_by_file
                .remove(&file.policy.filename)
                .unwrap_or_default(),
        );
        raw.incomplete_rules.extend(
            cross
                .incomplete
                .remove(&file.policy.filename)
                .unwrap_or_default(),
        );
        if !index.complete {
            raw.incomplete_rules
                .extend(rules::cross_file_rules().map(|rule| rule.code.to_owned()));
        }
        let result = raw.finish(&file.document, &file.policy.filename);
        snapshot
            .errors
            .extend(result.errors.into_iter().map(|message| InputError {
                filename: file.policy.filename.clone(),
                message,
            }));
        diagnostics.extend(result.diagnostics.into_iter().filter(|diagnostic| {
            snapshot.selected.contains(&diagnostic.filename)
                || (rules::rule(&diagnostic.code).is_some_and(|rule| rule.requires_index)
                    && diagnostic.related.iter().any(|related| {
                        snapshot.selected.contains(&related.filename)
                            || (diagnostic.code == "PTR002"
                                && snapshot.requested.iter().any(|selected| {
                                    index.root.join(&related.filename).starts_with(selected)
                                }))
                    }))
        }));
        suppressions.insert(file.policy.filename.clone(), result.suppressions);
    }
    snapshot.index.set_suppressions(suppressions);
    snapshot.sort_errors();
    Ok(Analysis {
        snapshot,
        diagnostics: sorted_diagnostics(&diagnostics),
    })
}
