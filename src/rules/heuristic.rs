use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::normative::constrained;
use crate::config::Config;
use crate::diagnostics::{Diagnostic, RelatedLocation, Span};
use crate::md::prose::{assertions, marker, runs};
use crate::md::{BlockKind, Document, FragmentKind, Language, Sentence};
use crate::sections::{self, SectionType, contains, is_quoted, main_flow, prose_text, rationale};

pub const PREAMBLE_LINES: usize = 30;
pub const PREAMBLE_TABLE_LINES: usize = 10;
pub const RATIONALE_PARAGRAPHS: usize = 2;
pub const RATIONALE_CHARS: usize = 160;

static VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bv?[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:[-+][a-z0-9]+(?:[.-][a-z0-9]+)*)?\b")
        .unwrap()
});
static COMPARISON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[<>≤≥~=^]|\d+(?:\.\d+)+\s*(?:\+|[-–]))").unwrap());
static VERSION_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:version|release)\s*[:=]?\s*v?[0-9]+\.").unwrap());
static MEASUREMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9]+(?:\.[0-9]+)?\s*(?:%|ms\b|seconds?\b|requests?/s\b|ops/s\b|mb/s\b|gb/s\b|倍|秒)").unwrap()
});

fn conditional(text: &str) -> bool {
    contains(
        text,
        &[
            "if",
            "when",
            "unless",
            "once",
            "until",
            "after",
            "before",
            "whether",
            "verify",
            "check",
            "ensure",
            "confirm",
            "must",
            "should",
            "will",
            "would",
            "can be",
            "may be",
            "如果",
            "若",
            "当",
            "部署后",
            "部署前",
            "检查",
            "确认",
            "确保",
            "应当",
            "必须",
            "将",
            "可能",
            "是否",
            "場合",
            "とき",
            "なら",
            "確認",
            "必要",
            "してください",
            "予定",
            "かどうか",
        ],
    )
}

fn deployment(language: Language) -> &'static [&'static str] {
    match language {
        Language::En => &[
            "is deployed",
            "are deployed",
            "is currently deployed",
            "are currently deployed",
            "has been deployed",
            "have been deployed",
            "is live",
            "is running in production",
            "is not yet deployed",
            "has not been deployed",
            "not yet shipped",
        ],
        Language::Zh => &[
            "已部署",
            "已经部署",
            "已上线",
            "已经上线",
            "尚未部署",
            "尚未上线",
            "正在生产环境运行",
        ],
        Language::Ja => &[
            "デプロイ済み",
            "デプロイされています",
            "本番稼働中",
            "まだデプロイされていません",
            "リリース済み",
        ],
    }
}

fn excluded_heading(language: Language) -> &'static [&'static str] {
    match language {
        Language::En => &[
            "what this does not",
            "what we did not",
            "what is not included",
            "out of scope",
            "non-goals",
            "without the",
        ],
        Language::Zh => &[
            "不包含的",
            "不包括的",
            "不在范围",
            "本次不做",
            "非目标",
            "未包含",
        ],
        Language::Ja => &["対象外", "含まれない", "今回実施しない", "非目標"],
    }
}

fn production_heading(language: Language) -> &'static [&'static str] {
    match language {
        Language::En => &[
            "implementation notes",
            "implementation approach",
            "what i changed",
            "what we changed",
            "changes made",
            "how i built",
            "how we built",
            "my approach",
        ],
        Language::Zh => &[
            "实现过程",
            "实现思路",
            "本次修改",
            "我做的修改",
            "修改说明",
            "制作过程",
        ],
        Language::Ja => &[
            "実装メモ",
            "実装方針",
            "今回の変更",
            "作成過程",
            "変更した内容",
        ],
    }
}

/// First-person production accounts. Statements that a page is generated or
/// maintained by someone are ownership notices, not narration.
fn narration(language: Language) -> &'static [&'static str] {
    match language {
        Language::En => &[
            "i have implemented",
            "i implemented",
            "i have added",
            "i updated",
            "i created this",
            "this page demonstrates",
        ],
        Language::Zh => &[
            "我已经实现",
            "我已实现",
            "我添加了",
            "我修改了",
            "本页面展示了实现过程",
        ],
        Language::Ja => &[
            "私は実装しました",
            "私が追加した",
            "このページを作成しました",
        ],
    }
}

fn evaluation(language: Language) -> &'static [&'static str] {
    match language {
        Language::En => &[
            "is faster",
            "is the fastest",
            "is better",
            "is the best",
            "is superior",
            "is more efficient",
            "is recommended",
            "we recommend",
            "recommended approach",
            "best performance",
        ],
        Language::Zh => &[
            "更快",
            "性能更好",
            "最高效",
            "最佳方案",
            "最好的",
            "推荐使用",
            "我们推荐",
        ],
        Language::Ja => &[
            "より高速",
            "最も高速",
            "最適な方法",
            "最高の性能",
            "推奨します",
            "を推奨",
        ],
    }
}

fn enabled_language(config: &Config, language: Language) -> bool {
    config
        .settings
        .lint
        .languages
        .iter()
        .any(|value| value == language.as_str())
}

fn enumerated_term(document: &Document, span: Span) -> bool {
    let before = document.source[..span.start].trim_end().chars().next_back();
    let after = document.source[span.end..].trim_start().chars().next();
    before.is_some_and(|ch| ",，、(（".contains(ch))
        && after.is_some_and(|ch| ",，、)）".contains(ch))
}

fn has_evidence(document: &Document, sentence: &Sentence, recommendation: bool) -> bool {
    let block = &document.blocks[sentence.block];
    if recommendation
        && contains(
            &sections::block_text(document, block),
            &[
                "because",
                "since",
                "so that",
                "to avoid",
                "to ensure",
                "consistency",
                "this avoids",
                "this prevents",
                "因为",
                "为了",
                "避免",
                "一致性",
                "区分",
                "一貫性",
                "区別",
                "ため",
                "防ぐ",
            ],
        )
    {
        return true;
    }
    // Nearby means this block or one adjacent direct-content block in the same section.
    let blocks: Vec<_> = document.sections[block.section]
        .blocks
        .iter()
        .copied()
        .filter(|&id| {
            let candidate = &document.blocks[id];
            candidate.parent == block.parent
                && !is_quoted(document, candidate)
                && !matches!(
                    candidate.kind,
                    BlockKind::Heading | BlockKind::HtmlComment | BlockKind::ThematicBreak
                )
        })
        .collect();
    let Some(position) = blocks.iter().position(|&id| id == sentence.block) else {
        return false;
    };
    blocks[position.saturating_sub(1)..(position + 2).min(blocks.len())]
        .iter()
        .any(|&id| {
            let candidate = &document.blocks[id];
            if recommendation
                && candidate.span.start > block.span.end
                && main_flow(document, candidate)
            {
                return true;
            }
            document
                .links
                .iter()
                .any(|link| !link.image && candidate.span.contains(link.span))
                || document
                    .sentences
                    .iter()
                    .filter(|value| {
                        candidate.span.contains(value.span)
                            && !is_quoted(document, &document.blocks[value.block])
                    })
                    .any(|sentence| {
                        let text = runs(sentence, true)
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        MEASUREMENT.is_match(&text)
                            || contains(
                                &text,
                                &[
                                    "measured",
                                    "benchmark",
                                    "benchmarks",
                                    "measurement",
                                    "测试结果",
                                    "基准测试",
                                    "实测",
                                    "ベンチマーク",
                                    "測定結果",
                                ],
                            )
                            || sentence.fragments.iter().any(|fragment| {
                                fragment.kind == FragmentKind::Text && fragment.text.contains("[^")
                            })
                    })
        })
}

#[derive(Default)]
pub struct HeuristicResult {
    pub diagnostics: Vec<Diagnostic>,
    pub incomplete_rules: BTreeSet<String>,
}

pub fn check(
    document: &Document,
    filename: &str,
    config: &Config,
    enabled: &BTreeSet<String>,
) -> HeuristicResult {
    let mut findings = Findings {
        document,
        filename,
        config,
        enabled,
        result: HeuristicResult::default(),
    };
    if ["STL002", "STL004", "VOX002", "VOX003", "EVD001"]
        .iter()
        .any(|code| enabled.contains(*code))
    {
        findings.check_sentences();
    }
    if enabled_language(config, document.language)
        && ["RAT001", "ORD001", "ORD002", "MIX001"]
            .iter()
            .any(|code| enabled.contains(*code))
    {
        findings.check_structure();
    }
    findings.result
}

struct Findings<'a> {
    document: &'a Document,
    filename: &'a str,
    config: &'a Config,
    enabled: &'a BTreeSet<String>,
    result: HeuristicResult,
}

impl Findings<'_> {
    /// Callers check that the rule is enabled before evaluating its conditions.
    fn emit(&mut self, code: &str, span: Span, message: &str, suggestion: &str) -> &mut Diagnostic {
        debug_assert!(self.enabled.contains(code), "{code} is not enabled");
        self.result.diagnostics.push(Diagnostic::new(
            self.filename,
            &self.document.source,
            code,
            span,
            message,
            suggestion,
        ));
        self.result.diagnostics.last_mut().unwrap()
    }

    fn emit_before_flow(
        &mut self,
        code: &str,
        span: Span,
        message: &str,
        suggestion: &str,
        first: Span,
    ) {
        let related = RelatedLocation::new(
            self.filename,
            &self.document.source,
            first,
            "The main flow starts here.",
        );
        self.emit(code, span, message, suggestion)
            .related
            .push(related);
    }

    fn check_sentences(&mut self) {
        for sentence in &self.document.sentences {
            let block = &self.document.blocks[sentence.block];
            if !enabled_language(self.config, sentence.language)
                || is_quoted(self.document, block)
                || sections::example_section(self.document, block.section)
            {
                continue;
            }
            let prose = assertions(sentence, false);
            let text = prose
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let heading = block.kind == BlockKind::Heading;
            self.check_deployment(sentence, &prose, &text);
            self.check_versions(sentence, &text);
            self.check_excluded_scope(sentence, &prose, heading);
            self.check_narration(sentence, &prose, heading);
            self.check_evidence(sentence, &prose, &text, heading);
        }
    }

    fn check_deployment(
        &mut self,
        sentence: &Sentence,
        prose: &[crate::md::prose::Run<'_>],
        text: &str,
    ) {
        if self.enabled.contains("STL002")
            && !conditional(text)
            && let Some(span) = marker(prose, deployment(sentence.language))
        {
            let state_text = assertions(sentence, true)
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let state = sentence.language != Language::En
                || contains(
                    &state_text,
                    &[
                        "currently",
                        "now",
                        "already",
                        "has been deployed",
                        "have been deployed",
                        "has not been deployed",
                        "not yet",
                        "is live",
                        "in production",
                        "to production",
                    ],
                );
            if !state && self.enabled.contains("STL002") {
                self.result.incomplete_rules.insert("STL002".into());
            }
            if state {
                self.emit(
                    "STL002",
                    span,
                    "This sentence asserts a deployment state in a long-lived page.",
                    "Replace the state with the command or dashboard that checks it; keep dated results in a release record.",
                );
            }
        }
    }

    fn check_versions(&mut self, sentence: &Sentence, text: &str) {
        if self.enabled.contains("STL004")
            && !constrained(sentence, self.config)
            && !conditional(text)
            && !contains(
                text,
                &[
                    "versioned",
                    "compatibility",
                    "compatible",
                    "introduced in",
                    "added in",
                    "deprecated in",
                    "removed in",
                    "fixed in",
                    "license",
                    "许可证",
                    "ライセンス",
                    "中得到解决",
                    "since",
                    "starting with",
                    "example",
                    "such as",
                    "or higher",
                    "or greater",
                    "or earlier",
                    "or lower",
                    "prior to",
                    "older than",
                    "newer than",
                    "up to",
                    "for example",
                    "e.g.",
                    "支持",
                    "兼容",
                    "引入",
                    "弃用",
                    "例如",
                    "対応",
                    "以降",
                    "導入",
                    "例",
                    "以前",
                    "より前",
                ],
            )
        {
            for run in assertions(sentence, true) {
                let label = VERSION_LABEL.is_match(&run.text);
                let snapshot = label
                    || contains(
                        text,
                        &[
                            "version is",
                            "version:",
                            "release is",
                            "release:",
                            "runs version",
                            "running version",
                            "we use version",
                            "版本为",
                            "版本是",
                            "版本：",
                            "バージョンは",
                            "バージョン:",
                        ],
                    );
                if !snapshot || COMPARISON.is_match(&run.text) {
                    continue;
                }
                for value in VERSION.find_iter(&run.text) {
                    let left = run.text[..value.start()].chars().next_back();
                    let right = run.text[value.end()..].chars().next();
                    if left.is_some_and(|ch| {
                        (ch.is_alphanumeric() || "_./\\:-".contains(ch)) && !(label && ch == ':')
                    }) || right.is_some_and(|ch| ch.is_alphanumeric() || "_/\\".contains(ch))
                        || (right == Some('.')
                            && run.text[value.end() + 1..]
                                .starts_with(|ch: char| ch.is_alphanumeric()))
                    {
                        continue;
                    }
                    // Require an explicit version label, avoiding decimals, IPs and model names.
                    if !contains(text, &["version", "release", "版本", "バージョン"])
                        && !value.as_str().starts_with(['v', 'V'])
                    {
                        continue;
                    }
                    if let Some(span) = run.span(Span::new(value.start(), value.end())) {
                        self.emit(
                            "STL004",
                            span,
                            "This version has no lasting constraint or historical context.",
                            "State the supported version range, or link to the manifest or release record that owns the version.",
                        );
                    }
                }
            }
        }
    }

    fn check_excluded_scope(
        &mut self,
        sentence: &Sentence,
        prose: &[crate::md::prose::Run<'_>],
        heading: bool,
    ) {
        if self.enabled.contains("VOX002")
            && heading
            && let Some(span) = marker(prose, excluded_heading(sentence.language))
        {
            self.emit(
                "VOX002",
                span,
                "This heading frames the section around excluded scope.",
                "Name the capability or boundary readers need; keep project non-goals in a plan or ADR.",
            );
        }
    }

    fn check_narration(
        &mut self,
        sentence: &Sentence,
        prose: &[crate::md::prose::Run<'_>],
        heading: bool,
    ) {
        if self.enabled.contains("VOX003")
            && let Some(span) = marker(
                prose,
                if heading {
                    production_heading(sentence.language)
                } else {
                    narration(sentence.language)
                },
            )
        {
            self.emit(
                "VOX003",
                span,
                "This text narrates how the deliverable was produced.",
                "Describe the resulting behavior or procedure; move implementation history to an ADR, plan, or change record.",
            );
        }
    }

    fn check_evidence(
        &mut self,
        sentence: &Sentence,
        prose: &[crate::md::prose::Run<'_>],
        text: &str,
        heading: bool,
    ) {
        if self.enabled.contains("EVD001")
            && !heading
            && !conditional(text)
            && let Some(span) = marker(prose, evaluation(sentence.language))
            && !enumerated_term(self.document, span)
            && !has_evidence(
                self.document,
                sentence,
                contains(text, &["recommend", "recommended", "推荐", "推奨"])
                    && !contains(
                        text,
                        &[
                            "faster",
                            "fastest",
                            "better",
                            "best",
                            "superior",
                            "efficient",
                            "更快",
                            "性能",
                            "最高效",
                            "高速",
                        ],
                    ),
            )
        {
            let generic_recommendation =
                contains(text, &["recommend", "recommended", "推荐", "推奨"])
                    && !contains(
                        text,
                        &[
                            "faster",
                            "fastest",
                            "better",
                            "best",
                            "superior",
                            "efficient",
                            " over ",
                            "rather than",
                            "更快",
                            "性能",
                            "最高效",
                            "高速",
                            "优于",
                        ],
                    );
            if generic_recommendation {
                self.result.incomplete_rules.insert("EVD001".into());
            } else {
                self.emit(
                    "EVD001",
                    span,
                    "This evaluative claim has no nearby measurement or source.",
                    "Link the source, or the dated record that holds the measurement, in this or an adjacent block, or replace the judgment with the specific behavior.",
                );
            }
        }
    }

    fn check_structure(&mut self) {
        let annotations = sections::classify(self.document);
        let opaque = sections::opaque_flow(self.document);
        if self.enabled.contains("MIX001") {
            self.check_mixed_sections(&annotations);
            if opaque.is_some() {
                self.result.incomplete_rules.insert("MIX001".into());
            }
        }
        if !["RAT001", "ORD001", "ORD002"]
            .iter()
            .any(|code| self.enabled.contains(*code))
        {
            return;
        }
        let first = self.document.blocks.iter().find(|block| {
            main_flow(self.document, block)
                && !in_recovery(self.document, &annotations, block.section)
        });
        let uncertain_order = first.is_none()
            || opaque.is_some_and(|span| first.is_none_or(|first| span.start < first.span.start))
            || self.document.blocks.iter().any(|block| {
                first.is_none_or(|first| block.span.start < first.span.start)
                    && !in_recovery(self.document, &annotations, block.section)
                    && sections::unclassified_example(self.document, block)
            });
        if uncertain_order {
            self.result.incomplete_rules.extend(
                ["ORD001", "ORD002", "RAT001"]
                    .into_iter()
                    .filter(|code| self.enabled.contains(*code))
                    .map(str::to_owned),
            );
        }
        if let Some(first) = first.filter(|_| !uncertain_order) {
            let preceding: Vec<_> = self
                .document
                .blocks
                .iter()
                .filter(|block| {
                    block.span.end <= first.span.start && !is_quoted(self.document, block)
                })
                .collect();
            self.check_rationale_preamble(first, &preceding);
            self.check_long_preamble(first, &preceding);
            self.check_recovery_order(first, &annotations);
        }
    }

    fn check_rationale_preamble(
        &mut self,
        first: &crate::md::Block,
        preceding: &[&crate::md::Block],
    ) {
        if self.enabled.contains("RAT001") {
            let rationale_blocks: Vec<_> = preceding
                .iter()
                .filter(|block| rationale_paragraph(self.document, block, self.config))
                .collect();
            let chars: usize = rationale_blocks
                .iter()
                .flat_map(|block| &block.sentences)
                .map(|&id| prose_text(&self.document.sentences[id]).chars().count())
                .sum();
            if rationale_blocks.len() >= RATIONALE_PARAGRAPHS && chars >= RATIONALE_CHARS {
                self.emit_before_flow(
                    "RAT001",
                    rationale_blocks[0].span,
                    "Design-choice arguments precede the first procedure or runnable example.",
                    "Move the extended rationale to an ADR and link to it after the main steps.",
                    first.span,
                );
            }
        }
    }

    fn check_long_preamble(&mut self, first: &crate::md::Block, preceding: &[&crate::md::Block]) {
        if self.enabled.contains("ORD001") {
            // Count occupied content lines, not YAML, blank lines, comments, headings or fenced examples.
            let mut lines = BTreeSet::new();
            let mut long_table = None;
            for block in preceding {
                if !matches!(block.kind, BlockKind::Paragraph | BlockKind::Table) {
                    continue;
                }
                let text = &self.document.source[block.span.start..block.span.end];
                let count = text.lines().filter(|line| !line.trim().is_empty()).count();
                if block.kind == BlockKind::Table && count > PREAMBLE_TABLE_LINES {
                    long_table = Some(block.span);
                }
                let start = self.document.source[..block.span.start]
                    .bytes()
                    .filter(|&b| b == b'\n')
                    .count();
                for (offset, line) in text.lines().enumerate() {
                    if !line.trim().is_empty() {
                        lines.insert(start + offset);
                    }
                }
            }
            if lines.len() > PREAMBLE_LINES || long_table.is_some() {
                let span = long_table
                    .or_else(|| {
                        preceding
                            .iter()
                            .find(|block| block.kind == BlockKind::Paragraph)
                            .map(|block| block.span)
                    })
                    .unwrap_or(first.span);
                self.emit_before_flow(
                    "ORD001",
                    span,
                    "A long preamble delays the first procedure or runnable example.",
                    "Put the runnable example or ordered procedure before extended background and reference tables.",
                    first.span,
                );
            }
        }
    }

    fn check_recovery_order(
        &mut self,
        first: &crate::md::Block,
        annotations: &[sections::SectionAnnotation],
    ) {
        if self.enabled.contains("ORD002") {
            for annotation in annotations {
                let section = &self.document.sections[annotation.section];
                if annotation.section_type == SectionType::Troubleshooting
                    && section
                        .heading_span
                        .is_some_and(|span| span.start < first.span.start)
                    && section
                        .parent
                        .is_none_or(|parent| !in_recovery(self.document, annotations, parent))
                {
                    self.emit_before_flow(
                        "ORD002",
                        section.heading_span.unwrap(),
                        "Troubleshooting or exceptions appear before the main flow.",
                        "Move this section after the main procedure so readers can reach the common path first.",
                        first.span,
                    );
                }
            }
        }
    }

    fn check_mixed_sections(&mut self, annotations: &[sections::SectionAnnotation]) {
        for annotation in annotations {
            let section = &self.document.sections[annotation.section];
            if annotation.section_type != SectionType::Rationale || section.heading_span.is_none() {
                continue;
            }
            if !section
                .heading
                .as_deref()
                .is_some_and(sections::decision_heading)
                && !section.blocks.iter().any(|&id| {
                    let block = &self.document.blocks[id];
                    rationale_paragraph(self.document, block, self.config)
                })
            {
                continue;
            }
            let paragraphs = section
                .blocks
                .iter()
                .filter(|&&id| {
                    self.document.blocks[id].kind == BlockKind::Paragraph
                        && !is_quoted(self.document, &self.document.blocks[id])
                })
                .count();
            if paragraphs >= RATIONALE_PARAGRAPHS {
                self.emit(
                    "MIX001",
                    section.heading_span.unwrap(),
                    "An extended rationale section conflicts with this procedure or reference page.",
                    "Move the decision rationale to an ADR and keep a link beside the procedure or contract it explains.",
                );
            }
        }
    }
}

fn in_recovery(
    document: &Document,
    annotations: &[sections::SectionAnnotation],
    mut id: usize,
) -> bool {
    loop {
        if annotations[id].section_type == SectionType::Troubleshooting {
            return true;
        }
        let Some(parent) = document.sections[id].parent else {
            return false;
        };
        id = parent;
    }
}

fn rationale_paragraph(document: &Document, block: &crate::md::Block, config: &Config) -> bool {
    block.kind == BlockKind::Paragraph
        && block.sentences.iter().any(|&id| {
            let sentence = &document.sentences[id];
            enabled_language(config, sentence.language)
                && rationale(&prose_text(sentence), sentence.language)
        })
}
