//! Read-only workspace rules. Every comparison is between two observed blocks.

use crate::rules::Kind;

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, RelatedLocation, Span, sorted_diagnostics};
use crate::index::{IndexedFile, LinkStatus, WorkspaceIndex};
use crate::md::{BlockKind, Document, FragmentKind, Language};
use crate::paths::normalize;

const DUPLICATION_RULES: [&str; 5] = ["DUP001", "DUP002", "DUP003", "OWN001", "OWN002"];

#[derive(Debug, Default)]
pub struct CrossReport {
    pub diagnostics: Vec<Diagnostic>,
    pub incomplete: BTreeMap<String, BTreeSet<String>>,
    pub errors: Vec<(String, String)>,
}

pub fn check(index: &WorkspaceIndex) -> CrossReport {
    let mut report = CrossReport::default();
    for file in index.files() {
        if !index.complete {
            for code in DUPLICATION_RULES {
                mark_incomplete(&mut report, file, code);
            }
        }
        check_links(index, file, &mut report);
    }
    let active = |code| index.files().iter().any(|file| enabled(file, code));
    // OWN002 identifies ties from every comparison family.
    let ownership = active("OWN002");
    if active("DUP001") || active("OWN001") || ownership {
        let definitions = definition_units(index);
        if active("DUP001") || ownership {
            let edges = similarity_edges(index, &definitions, false, false);
            emit_owned(index, &definitions, &edges, "DUP001", &mut report);
        }
        if active("OWN001") || ownership {
            let edges = similarity_edges(index, &definitions, true, true);
            emit_owned(index, &definitions, &edges, "OWN001", &mut report);
        }
    }
    if active("DUP002") || ownership {
        let sections = section_units(index);
        let edges = restatement_edges(index, &sections, &mut report);
        emit_owned(index, &sections, &edges, "DUP002", &mut report);
    }
    if active("DUP003") || ownership {
        let paragraphs = paragraph_units(index);
        let edges = similarity_edges(index, &paragraphs, false, false);
        emit_owned(index, &paragraphs, &edges, "DUP003", &mut report);
    }
    // A tied definition can participate in more than one duplication rule.
    // Merge related locations, retaining the directly observed pairwise evidence.
    let mut merged: BTreeMap<(String, Span, String), Diagnostic> = BTreeMap::new();
    for diagnostic in report.diagnostics {
        let key = (
            diagnostic.filename.clone(),
            diagnostic.byte_range,
            diagnostic.code.clone(),
        );
        if let Some(existing) = merged.get_mut(&key) {
            existing.related.extend(diagnostic.related);
        } else {
            merged.insert(key, diagnostic);
        }
    }
    for diagnostic in merged.values_mut() {
        diagnostic.related.sort();
        diagnostic.related.dedup();
    }
    report.diagnostics = sorted_diagnostics(&merged.into_values().collect::<Vec<_>>());
    report.errors.sort();
    report.errors.dedup();
    report
}

fn enabled(file: &IndexedFile, code: &str) -> bool {
    file.policy.kind_value() != Some(Kind::Generated)
        && file.policy.enabled_rules.iter().any(|item| item == code)
}

fn mark_incomplete(report: &mut CrossReport, file: &IndexedFile, code: &str) {
    if enabled(file, code) {
        report
            .incomplete
            .entry(file.policy.filename.clone())
            .or_default()
            .insert(code.to_owned());
    }
}

fn check_links(index: &WorkspaceIndex, file: &IndexedFile, report: &mut CrossReport) {
    if !enabled(file, "LNK002") && !enabled(file, "PTR002") {
        return;
    }
    for link in &file.document.links {
        let resolution = index.resolve_link(&file.policy.filename, &link.destination);
        match resolution.status {
            LinkStatus::AnchorMissing if enabled(file, "LNK002") => {
                let target = resolution
                    .target
                    .as_deref()
                    .and_then(|name| index.file(name));
                if let Some(target) = target {
                    let mut diagnostic = Diagnostic::new(
                        &file.policy.filename,
                        &file.document.source,
                        "LNK002",
                        link.span,
                        format!(
                            "Anchor {:?} does not exist in {}.",
                            resolution.anchor.as_deref().unwrap_or_default(),
                            target.policy.filename
                        ),
                        "Update the fragment to an existing heading or explicit HTML anchor in the target document.",
                    );
                    diagnostic.related.push(related(
                        target,
                        Span::new(0, 0),
                        "The target document has no matching anchor.",
                    ));
                    report.diagnostics.push(diagnostic);
                }
            }
            LinkStatus::Directory if enabled(file, "PTR002") => {
                let Some(target) = resolution.target.as_deref() else {
                    continue;
                };
                let path = index.root.join(target);
                let allowed = file
                    .config
                    .settings
                    .lint
                    .ptr
                    .catalog_dirs
                    .iter()
                    .any(|catalog| {
                        normalize(file.config.directory.join(catalog.trim_end_matches('/'))) == path
                    });
                if !allowed {
                    let mut diagnostic = Diagnostic::new(
                        &file.policy.filename,
                        &file.document.source,
                        "PTR002",
                        link.span,
                        format!(
                            "Link points to the directory {target:?} without a declared catalog."
                        ),
                        "Link to the document that answers the reader's question, or declare this directory in lint.ptr.catalog-dirs if it is a maintained index.",
                    );
                    diagnostic.related.push(RelatedLocation::new(
                        target,
                        "",
                        Span::new(0, 0),
                        "Directory target.",
                    ));
                    report.diagnostics.push(diagnostic);
                }
            }
            LinkStatus::AnchorUnknown => {
                mark_incomplete(report, file, "LNK002");
                if resolution.target.is_none() {
                    mark_incomplete(report, file, "PTR002");
                }
            }
            LinkStatus::Template | LinkStatus::OutsideWorkspace | LinkStatus::Unreadable => {
                mark_incomplete(report, file, "LNK002");
                mark_incomplete(report, file, "PTR002");
                if let Some(error) = resolution.error {
                    report.errors.push((file.policy.filename.clone(), error));
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
struct Unit {
    file: usize,
    span: Span,
    values: BTreeSet<String>,
    heads: BTreeSet<String>,
    paragraph: bool,
    whole_document: bool,
}

fn identifiers(document: &Document, span: Span) -> BTreeSet<String> {
    document
        .identifiers
        .iter()
        .filter(|id| span.contains(id.span))
        .map(|id| id.text.clone())
        .collect()
}

fn definition_units(index: &WorkspaceIndex) -> Vec<Unit> {
    let mut units = Vec::new();
    for (file_index, file) in index.files().iter().enumerate() {
        let document = &file.document;
        for (block_index, block) in document.blocks.iter().enumerate() {
            let mut heads = BTreeSet::new();
            match block.kind {
                BlockKind::Table => {
                    // The header row is a schema label, not a field definition.
                    for row in block.children.iter().skip(1) {
                        let cells = &document.blocks[*row].children;
                        if let Some(first) = cells.first() {
                            let described = cells.iter().skip(1).any(|cell| {
                                let span = document.blocks[*cell].span;
                                document.source[span.start..span.end]
                                    .chars()
                                    .any(char::is_alphanumeric)
                            });
                            if described {
                                heads.extend(identifiers(document, document.blocks[*first].span));
                            }
                        }
                    }
                }
                BlockKind::List => {
                    // An outer definition list owns nested descriptions once.
                    if ancestors(document, block_index)
                        .any(|parent| document.blocks[parent].kind == BlockKind::List)
                    {
                        continue;
                    }
                    for item in &block.children {
                        let Some(paragraph) = document.blocks[*item].children.first() else {
                            continue;
                        };
                        if document.blocks[*paragraph].kind != BlockKind::Paragraph {
                            continue;
                        }
                        let span = document.blocks[*paragraph].span;
                        let Some(identifier) = document
                            .identifiers
                            .iter()
                            .find(|id| span.contains(id.span))
                        else {
                            continue;
                        };
                        // A definition starts with the key; a list of mentions or bare keys does not define it.
                        let before = document.source[span.start..identifier.span.start]
                            .trim_matches([' ', '\t', '*', '_']);
                        let after = &document.source[identifier.span.end..span.end];
                        let prose = document.blocks[*paragraph]
                            .sentences
                            .iter()
                            .flat_map(|sentence| &document.sentences[*sentence].fragments)
                            .filter(|fragment| {
                                fragment.span.start >= identifier.span.end
                                    && matches!(
                                        fragment.kind,
                                        FragmentKind::Text | FragmentKind::LinkText
                                    )
                            })
                            .map(|fragment| fragment.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        let description = after
                            .trim_start()
                            .starts_with([':', '：', '-', '—', '–', '='])
                            && after.chars().any(char::is_alphanumeric)
                            || prose
                                .split(|character: char| !character.is_alphanumeric())
                                .any(|word| {
                                    !word.is_empty()
                                        && !["and", "or", "及", "和", "または"].contains(&word)
                                });
                        if before.is_empty() && description {
                            heads.insert(identifier.text.clone());
                        }
                    }
                }
                _ => continue,
            }
            if !heads.is_empty() {
                units.push(Unit {
                    file: file_index,
                    span: block.span,
                    values: identifiers(document, block.span),
                    heads,
                    paragraph: false,
                    whole_document: false,
                });
            }
        }
    }
    units
}

fn ancestors(document: &Document, block: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors(document.blocks[block].parent, |parent| {
        document.blocks[*parent].parent
    })
}

fn section_units(index: &WorkspaceIndex) -> Vec<Unit> {
    let mut units = Vec::new();
    for (file_index, file) in index.files().iter().enumerate() {
        for (section_index, section) in file.document.sections.iter().enumerate() {
            let values = file
                .document
                .identifiers
                .iter()
                .filter(|id| id.section == section_index)
                .map(|id| id.text.clone())
                .collect();
            // Only the section's own content participates, excluding child sections.
            let end = section.children.first().map_or(section.span.end, |child| {
                file.document.sections[*child].span.start
            });
            units.push(Unit {
                file: file_index,
                span: Span::new(section.span.start, end),
                values,
                heads: BTreeSet::new(),
                paragraph: false,
                whole_document: false,
            });
        }
        let span = Span::new(0, file.document.source.len());
        units.push(Unit {
            file: file_index,
            span,
            values: identifiers(&file.document, span),
            heads: BTreeSet::new(),
            paragraph: false,
            whole_document: true,
        });
    }
    units
}

fn paragraph_units(index: &WorkspaceIndex) -> Vec<Unit> {
    let mut units = Vec::new();
    for (file_index, file) in index.files().iter().enumerate() {
        let settings = &file.config.settings.lint.dup;
        for (block_index, block) in file.document.blocks.iter().enumerate() {
            if block.kind != BlockKind::Paragraph
                || ancestors(&file.document, block_index).any(|parent| {
                    matches!(
                        file.document.blocks[parent].kind,
                        BlockKind::List | BlockKind::Table | BlockKind::Blockquote
                    )
                })
            {
                continue;
            }
            let text = block
                .sentences
                .iter()
                .flat_map(|sentence| &file.document.sentences[*sentence].fragments)
                .filter(|fragment| {
                    matches!(
                        fragment.kind,
                        FragmentKind::Text | FragmentKind::LinkText | FragmentKind::InlineCode
                    )
                })
                .map(|fragment| fragment.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if text
                .chars()
                .filter(|character| character.is_alphanumeric())
                .count()
                < settings.min_paragraph_chars
            {
                continue;
            }
            let normalized: Vec<char> = text
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .collect();
            let values: BTreeSet<String> = normalized
                .windows(settings.shingle_size)
                .map(|window| window.iter().collect())
                .collect();
            // Repetitive punctuation and very low-information text are not evidence.
            if values.len() < 8 {
                continue;
            }
            units.push(Unit {
                file: file_index,
                span: block.span,
                values,
                heads: BTreeSet::new(),
                paragraph: true,
                whole_document: false,
            });
        }
    }
    units
}

fn language(language: Language) -> u8 {
    match language {
        Language::En => 0,
        Language::Zh => 1,
        Language::Ja => 2,
    }
}

fn same_domain(left: &IndexedFile, right: &IndexedFile) -> bool {
    left.policy.domain.as_deref().unwrap_or("") == right.policy.domain.as_deref().unwrap_or("")
        && left.document.language == right.document.language
}

type Edges = BTreeMap<usize, BTreeSet<usize>>;

fn similarity_edges(
    index: &WorkspaceIndex,
    units: &[Unit],
    heads_only: bool,
    plan_only: bool,
) -> Edges {
    let mut postings: BTreeMap<(&str, u8, &str), Vec<usize>> = BTreeMap::new();
    let mut frequencies: BTreeMap<(&str, u8, &str), usize> = BTreeMap::new();
    let mut minimum_threshold: BTreeMap<(&str, u8), f64> = BTreeMap::new();
    for unit in units {
        let file = &index.files()[unit.file];
        let domain = (
            file.policy.domain.as_deref().unwrap_or(""),
            language(file.document.language),
        );
        let settings = &file.config.settings.lint.dup;
        let threshold = if unit.paragraph {
            settings.min_paragraph_similarity
        } else {
            settings.min_jaccard
        };
        minimum_threshold
            .entry(domain)
            .and_modify(|minimum| *minimum = minimum.min(threshold))
            .or_insert(threshold);
        for value in if heads_only {
            &unit.heads
        } else {
            &unit.values
        } {
            *frequencies
                .entry((domain.0, domain.1, value.as_str()))
                .or_default() += 1;
        }
    }
    let mut edges = Edges::new();
    for (current, unit) in units.iter().enumerate() {
        let file = &index.files()[unit.file];
        let values = if heads_only {
            &unit.heads
        } else {
            &unit.values
        };
        let domain = (
            file.policy.domain.as_deref().unwrap_or(""),
            language(file.document.language),
        );
        let threshold = minimum_threshold[&domain];
        // A qualifying Jaccard pair must intersect in these prefixes under one
        // shared token ordering. Rarest-first ordering avoids common prose shingles.
        let mut prefix: Vec<_> = values.iter().map(String::as_str).collect();
        prefix.sort_by_key(|value| (frequencies[&(domain.0, domain.1, *value)], *value));
        let prefix_length = values
            .len()
            .saturating_sub((threshold * values.len() as f64).ceil() as usize)
            + 1;
        prefix.truncate(prefix_length);
        let mut candidates = BTreeSet::new();
        for value in &prefix {
            let key = (
                file.policy.domain.as_deref().unwrap_or(""),
                language(file.document.language),
                *value,
            );
            if let Some(previous) = postings.get(&key) {
                candidates.extend(previous.iter().copied());
            }
        }
        for previous in candidates {
            let other = &units[previous];
            let other_file = &index.files()[other.file];
            if unit.file == other.file
                || (plan_only
                    && file.policy.kind_value() != Some(Kind::Plan)
                    && other_file.policy.kind_value() != Some(Kind::Plan))
            {
                continue;
            }
            let other_values = if heads_only {
                &other.heads
            } else {
                &other.values
            };
            if (values.len().min(other_values.len()) as f64)
                < threshold * values.len().max(other_values.len()) as f64
            {
                continue;
            }
            let intersection = values.intersection(other_values).count();
            let union = values.len() + other_values.len() - intersection;
            let similarity = if union == 0 {
                0.0
            } else {
                intersection as f64 / union as f64
            };
            for (source, target, source_file) in
                [(current, previous, file), (previous, current, other_file)]
            {
                let settings = &source_file.config.settings.lint.dup;
                let qualifies = if unit.paragraph {
                    // Different shingle sizes do not describe the same comparison measure.
                    settings.shingle_size == file.config.settings.lint.dup.shingle_size
                        && settings.shingle_size == other_file.config.settings.lint.dup.shingle_size
                        && similarity >= settings.min_paragraph_similarity
                } else {
                    intersection >= settings.min_identifiers && similarity >= settings.min_jaccard
                };
                if qualifies {
                    edges.entry(source).or_default().insert(target);
                }
            }
        }
        for value in prefix {
            postings
                .entry((
                    file.policy.domain.as_deref().unwrap_or(""),
                    language(file.document.language),
                    value,
                ))
                .or_default()
                .push(current);
        }
    }
    edges
}

fn restatement_edges(index: &WorkspaceIndex, units: &[Unit], report: &mut CrossReport) -> Edges {
    let mut edges = Edges::new();
    for (source_index, unit) in units.iter().enumerate() {
        if unit.whole_document {
            continue;
        }
        let file = &index.files()[unit.file];
        if unit.values.len() < file.config.settings.lint.dup.min_identifiers {
            continue;
        }
        let Some(link) = file
            .document
            .links
            .iter()
            .filter(|link| !link.image && unit.span.contains(link.span))
            .max_by_key(|link| link.span.end)
        else {
            continue;
        };
        let tail = &file.document.source[link.span.end..unit.span.end];
        if !tail
            .chars()
            .all(|character| character.is_whitespace() || ".。!！?？;；".contains(character))
        {
            continue;
        }
        let resolution = index.resolve_link(&file.policy.filename, &link.destination);
        if matches!(
            resolution.status,
            LinkStatus::AnchorUnknown
                | LinkStatus::Unreadable
                | LinkStatus::Template
                | LinkStatus::OutsideWorkspace
        ) {
            mark_incomplete(report, file, "DUP002");
            mark_incomplete(report, file, "OWN002");
        }
        if !matches!(
            resolution.status,
            LinkStatus::File | LinkStatus::AnchorFound
        ) {
            continue;
        }
        let Some(target) = resolution
            .target
            .as_deref()
            .and_then(|target| index.file(target))
        else {
            continue;
        };
        if file.policy.filename == target.policy.filename || !same_domain(file, target) {
            continue;
        }
        let anchor_span = resolution
            .anchor
            .as_deref()
            .and_then(|anchor| index.anchor_span(&target.policy.filename, anchor));
        let target_section = if let Some(span) = anchor_span {
            units
                .iter()
                .enumerate()
                .filter(|(_, unit)| {
                    !unit.whole_document
                        && index.files()[unit.file].policy.filename == target.policy.filename
                        && unit.span.contains(span)
                })
                .min_by_key(|(_, unit)| unit.span.len())
        } else {
            // A link without a fragment addresses the complete target document.
            units.iter().enumerate().find(|(_, unit)| {
                index.files()[unit.file].policy.filename == target.policy.filename
                    && unit.whole_document
            })
        };
        let Some((target_index, target_unit)) = target_section else {
            continue;
        };
        let shared = unit.values.intersection(&target_unit.values).count();
        let ratio = shared as f64 / unit.values.len() as f64;
        if shared >= file.config.settings.lint.dup.min_identifiers
            && ratio >= file.config.settings.lint.dup.min_jaccard
        {
            edges.entry(source_index).or_default().insert(target_index);
            edges.entry(target_index).or_default().insert(source_index);
        }
    }
    edges
}

fn rank(file: &IndexedFile) -> u8 {
    if file
        .document
        .frontmatter
        .as_ref()
        .is_some_and(|frontmatter| {
            frontmatter.errors.is_empty() && frontmatter.canonical == Some(true)
        })
    {
        return 7;
    }
    match file.policy.kind_value() {
        Some(Kind::Generated) => 6,
        Some(Kind::Reference) => 5,
        Some(Kind::Adr) => 4,
        Some(Kind::Howto) => 3,
        Some(Kind::Readme) => 2,
        _ => 1,
    }
}

fn related(file: &IndexedFile, span: Span, message: &str) -> RelatedLocation {
    RelatedLocation::new(&file.policy.filename, &file.document.source, span, message)
}

fn emit_owned(
    index: &WorkspaceIndex,
    units: &[Unit],
    edges: &Edges,
    code: &str,
    report: &mut CrossReport,
) {
    for (source, neighbors) in edges {
        let unit = &units[*source];
        let file = &index.files()[unit.file];
        let priority = rank(file);
        let higher: Vec<_> = neighbors
            .iter()
            .filter(|neighbor| rank(&index.files()[units[**neighbor].file]) > priority)
            .copied()
            .collect();
        let tied: Vec<_> = neighbors
            .iter()
            .filter(|neighbor| rank(&index.files()[units[**neighbor].file]) == priority)
            .copied()
            .collect();
        let (rule, peers, message, suggestion) = if !higher.is_empty() {
            (
                code,
                higher,
                match code {
                    "DUP001" => {
                        "This definition block repeats identifiers defined in a higher-priority document."
                    }
                    "DUP002" => {
                        "This section repeats identifiers from a linked section instead of leaving a focused reference."
                    }
                    "DUP003" => {
                        "This paragraph closely repeats prose in a higher-priority document."
                    }
                    "OWN001" => "A plan and another document both define these identifiers.",
                    _ => unreachable!(),
                },
                "Keep the shared definition or explanation in its authoritative document and replace the repeated content with a focused link.",
            )
        } else if !tied.is_empty() {
            (
                "OWN002",
                tied,
                "Directly matching content has multiple owners at the same highest priority.",
                "Choose one authoritative document, declare canonical: true there if needed, and link to it from the other documents.",
            )
        } else {
            continue;
        };
        if !enabled(file, rule) {
            continue;
        }
        let mut diagnostic = Diagnostic::new(
            &file.policy.filename,
            &file.document.source,
            rule,
            unit.span,
            message,
            suggestion,
        );
        for peer in peers {
            let other = &units[peer];
            diagnostic.related.push(related(
                &index.files()[other.file],
                other.span,
                if rule == "OWN002" {
                    "Matching content with the same ownership priority."
                } else {
                    "Matching content with higher ownership priority."
                },
            ));
        }
        report.diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn prefix_candidates_preserve_exact_pairwise_jaccard_results() {
        let root = tempfile::tempdir().unwrap();
        for threshold in [0.0_f64, 0.2, 0.5, 0.8, 1.0] {
            for paragraph in [false, true] {
                if paragraph && threshold == 0.0 {
                    continue;
                }
                let mut files = Vec::new();
                let mut units = Vec::new();
                // Exhaustive subsets, unequal sizes, independent domains, and
                // two per-file thresholds exercise the lossless candidate bound.
                for mask in 1_u32..128 {
                    let values: BTreeSet<String> = (0..7)
                        .filter(|bit| mask & (1 << bit) != 0)
                        .map(|bit| format!("key{bit}"))
                        .collect();
                    let mut config = Config::defaults(root.path()).unwrap();
                    config.settings.lint.dup.min_identifiers = 2;
                    config.settings.lint.dup.min_jaccard = if mask % 2 == 0 {
                        threshold
                    } else {
                        threshold.max(0.7)
                    };
                    config.settings.lint.dup.min_paragraph_similarity =
                        config.settings.lint.dup.min_jaccard;
                    let filename = format!("{mask:03}.md");
                    let mut document = crate::md::parse("").unwrap();
                    document.language = if mask % 3 == 0 {
                        Language::Ja
                    } else {
                        Language::En
                    };
                    let mut file = IndexedFile::new(
                        filename.clone(),
                        root.path().join(&filename),
                        document.into(),
                        config,
                        &crate::config::CliOverrides::default(),
                    )
                    .unwrap();
                    file.policy.kind = Some(crate::rules::KindOutcome::Mapped(Kind::Reference));
                    file.policy.domain = Some(((mask / 2) % 2).to_string());
                    file.policy.enabled_rules.clear();
                    files.push(file);
                    units.push(Unit {
                        file: units.len(),
                        span: Span::new(0, 0),
                        heads: values.clone(),
                        values,
                        paragraph,
                        whole_document: false,
                    });
                }
                let index = WorkspaceIndex::new(root.path().to_path_buf(), files, true);
                let actual = similarity_edges(&index, &units, false, false);
                let mut expected = Edges::new();
                for (left, source) in units.iter().enumerate() {
                    for (right, target) in units.iter().enumerate() {
                        if left == right
                            || !same_domain(&index.files()[left], &index.files()[right])
                        {
                            continue;
                        }
                        let shared = source.values.intersection(&target.values).count();
                        let union = source.values.union(&target.values).count();
                        let settings = &index.files()[left].config.settings.lint.dup;
                        let threshold = if paragraph {
                            settings.min_paragraph_similarity
                        } else {
                            settings.min_jaccard
                        };
                        if (paragraph || shared >= settings.min_identifiers)
                            && shared as f64 / union as f64 >= threshold
                        {
                            expected.entry(left).or_default().insert(right);
                        }
                    }
                }
                assert_eq!(
                    actual, expected,
                    "threshold={threshold}, paragraph={paragraph}"
                );
            }
        }
    }
}
