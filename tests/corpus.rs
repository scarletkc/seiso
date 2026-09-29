//! Robustness checks on unmodified pinned inputs, without precision labels.
//! The synthetic howto mapping exercises convention rules; it is not a genre annotation.
mod common;
use common::{CheckContext, check};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use seiso::config::{CliOverrides, Config};
use seiso::diagnostics::{Diagnostic, SourceMap, Span};
use seiso::rules::rule_codes;
use seiso::rules::suppression::SuppressionScope;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Corpus {
    schema_version: u32,
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    id: String,
    commit: String,
    documents: Vec<LockedDocument>,
}

#[derive(Deserialize)]
struct LockedDocument {
    path: String,
    git_blob: String,
    bytes: usize,
    sha256: String,
}

#[test]
#[ignore = "requires downloaded pinned corpus"]
fn pinned_corpus_rule_engine_is_deterministic_and_preserves_source_locations() {
    let corpus_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let lock = fs::read(corpus_root.join("corpus.lock.json")).expect("read pinned corpus lock");
    let mut corpus: Corpus = serde_json::from_slice(&lock).expect("parse pinned corpus lock");
    assert_eq!(corpus.schema_version, 1, "unsupported corpus schema");
    assert!(!corpus.sources.is_empty(), "corpus must contain sources");
    corpus.sources.sort_by(|left, right| left.id.cmp(&right.id));
    let mut seen_sources = BTreeSet::new();
    let mut seen_documents = BTreeSet::new();
    let mut totals: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    let mut checks = 0;
    let mut bytes = 0;
    let virtual_workspace = tempfile::tempdir().expect("create a virtual path root");
    let selected = rule_codes()
        .filter(|code| *code != "LNK001")
        .map(|code| format!("'{code}'"))
        .collect::<Vec<_>>()
        .join(",");
    let rules = format!("[lint]\nselect=[{selected}]\n");
    let overrides = CliOverrides {
        preview: true,
        ..CliOverrides::default()
    };
    for source in &mut corpus.sources {
        assert!(
            !source.id.is_empty()
                && source
                    .id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')),
            "unsafe corpus source identifier"
        );
        assert!(
            seen_sources.insert(source.id.clone()),
            "duplicate source {}",
            source.id
        );
        assert_hex(&source.commit, 40);
        assert!(!source.documents.is_empty(), "empty source {}", source.id);
        source
            .documents
            .sort_by(|left, right| left.path.cmp(&right.path));
        let root = virtual_workspace.path().join(&source.id);
        let declared = Config::parse(&rules, &root).expect("build declared-kind configuration");
        let synthetic = Config::parse(
            &format!("[[kinds]]\npath='**'\nkind='howto'\n{rules}"),
            &root,
        )
        .expect("build synthetic-kind configuration");
        for input in &source.documents {
            let identity = format!("{}/{}", source.id, input.path);
            assert!(
                seen_documents.insert(identity.clone()),
                "duplicate document {identity}"
            );
            assert!(
                !input.path.is_empty()
                    && !input.path.contains('\\')
                    && Path::new(&input.path)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "unsafe document path {identity}"
            );
            assert_hex(&input.git_blob, 40);
            assert_hex(&input.sha256, 64);
            let raw = fs::read(corpus_root.join("data/blobs").join(&input.git_blob))
                .unwrap_or_else(|error| panic!("Missing corpus input {identity}: {error}; run `python corpus/corpus.py fetch`"));
            assert_eq!(raw.len(), input.bytes, "changed size for {identity}");
            assert_eq!(
                format!("{:x}", Sha256::digest(&raw)),
                input.sha256,
                "changed content for {identity}"
            );
            bytes += raw.len();
            let text = String::from_utf8(raw)
                .unwrap_or_else(|error| panic!("Invalid UTF-8 in {identity}: {error}"));
            let document = seiso::md::parse(&text)
                .unwrap_or_else(|error| panic!("Cannot parse {identity}: {error}"));
            let map = SourceMap::new(&text);
            let path = root.join(&input.path);
            for (scenario, config) in [
                ("declared_kind", &declared),
                ("synthetic_howto", &synthetic),
            ] {
                let context = CheckContext {
                    document: &document,
                    filename: &identity,
                    path: &path,
                    workspace_root: &root,
                    config,
                    overrides: &overrides,
                };
                let first = check(&context)
                    .unwrap_or_else(|error| panic!("{identity} ({scenario}): {error}"));
                let second = check(&context)
                    .unwrap_or_else(|error| panic!("{identity} ({scenario}): {error}"));
                assert_eq!(
                    serde_json::to_vec(&(&first.diagnostics, &first.suppressions)).unwrap(),
                    serde_json::to_vec(&(&second.diagnostics, &second.suppressions)).unwrap(),
                    "nondeterministic diagnostics or suppressions for {identity} ({scenario})"
                );
                assert!(
                    first.errors.is_empty(),
                    "unexpected input errors for {identity}: {:?}",
                    first.errors
                );
                assert!(!first.enabled_rules.iter().any(|code| code == "LNK001"));
                let counts = totals.entry(scenario).or_default();
                for diagnostic in &first.diagnostics {
                    validate_diagnostic(diagnostic, &identity, &text, &map);
                    *counts.entry(diagnostic.code.clone()).or_default() += 1;
                }
                for suppression in &first.suppressions {
                    validate_span(suppression.span, &text, &identity);
                    if let Some(SuppressionScope::Block { span }) = suppression.scope {
                        validate_span(span, &text, &identity);
                    }
                }
                checks += 2;
            }
        }
    }
    assert!(!seen_documents.is_empty());
    println!(
        "Corpus robustness: {} repositories, {} documents, {bytes} verified bytes, {checks} engine checks; diagnostics by scenario: {}",
        seen_sources.len(),
        seen_documents.len(),
        serde_json::to_string(&totals).unwrap()
    );
}

fn assert_hex(value: &str, width: usize) {
    assert_eq!(
        value.len(),
        width,
        "unpinned or malformed content identifier"
    );
    assert!(
        value.chars().all(|ch| ch.is_ascii_hexdigit()),
        "malformed content identifier"
    );
}

fn validate_span(span: Span, source: &str, identity: &str) {
    assert!(span.start <= span.end, "reversed source span in {identity}");
    assert!(
        source.get(span.start..span.end).is_some(),
        "invalid UTF-8 source span {span:?} in {identity}"
    );
}

fn validate_diagnostic(diagnostic: &Diagnostic, identity: &str, source: &str, map: &SourceMap<'_>) {
    assert_eq!(diagnostic.filename, identity);
    validate_span(diagnostic.byte_range, source, identity);
    assert_eq!(
        (diagnostic.location, diagnostic.end_location),
        map.span_locations(diagnostic.byte_range),
        "invalid coordinates in {identity}"
    );
    assert!(
        diagnostic.fix.is_none(),
        "M1 diagnostics must not carry edits"
    );
    for related in &diagnostic.related {
        assert_eq!(related.filename, identity, "unexpected cross-file location");
        validate_span(related.byte_range, source, identity);
        assert_eq!(
            (related.location, related.end_location),
            map.span_locations(related.byte_range),
            "invalid related coordinates in {identity}"
        );
    }
}
