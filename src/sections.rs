//! Inspectable heuristic section roles. Classification is independent of file policy.

use crate::diagnostics::Span;
use crate::md::prose::{occurrences, runs};
use crate::md::{Block, BlockKind, Document, FragmentKind, Language, Sentence};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionType {
    Steps,
    Reference,
    Rationale,
    Background,
    Troubleshooting,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SectionAnnotation {
    pub section: usize,
    pub section_type: SectionType,
    /// Direct content only: a parent does not inherit a child's responsibility.
    pub content_span: Span,
    pub evidence: Vec<SectionEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SectionEvidence {
    pub signal: &'static str,
    pub span: Span,
}

pub(crate) fn contains(text: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| !occurrences(text, phrase).is_empty())
}

pub(crate) fn prose_text(sentence: &Sentence) -> String {
    runs(sentence, false)
        .iter()
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn block_text(document: &Document, block: &Block) -> String {
    block
        .sentences
        .iter()
        .map(|&id| prose_text(&document.sentences[id]))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn is_quoted(document: &Document, block: &Block) -> bool {
    let mut current = Some(block);
    while let Some(block) = current {
        if matches!(
            block.kind,
            BlockKind::Blockquote | BlockKind::FootnoteDefinition
        ) {
            return true;
        }
        current = block.parent.map(|id| &document.blocks[id]);
    }
    false
}

pub(crate) fn example_section(document: &Document, mut section: usize) -> bool {
    loop {
        let value = &document.sections[section];
        if value.heading.as_deref().is_some_and(|heading| {
            contains(
                heading,
                &[
                    "example output",
                    "sample output",
                    "example usage",
                    "sample conversation",
                    "example conversation",
                    "示例输出",
                    "输出示例",
                    "示例对话",
                    "使用示例",
                    "出力例",
                    "会話例",
                    "使用例",
                ],
            )
        }) {
            return true;
        }
        let Some(parent) = value.parent else {
            return false;
        };
        section = parent;
    }
}

fn output_section(document: &Document, mut section: usize) -> bool {
    loop {
        let value = &document.sections[section];
        let heading = value
            .heading
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if heading.ends_with(" output")
            || [
                "output",
                "assistant response",
                "sample conversation",
                "example conversation",
                "示例输出",
                "输出示例",
                "示例对话",
                "出力例",
                "会話例",
            ]
            .contains(&heading.as_str())
        {
            return true;
        }
        let Some(parent) = value.parent else {
            return false;
        };
        section = parent;
    }
}

fn starts(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| {
        occurrences(text.trim_start(), phrase)
            .first()
            .is_some_and(|span| span.start == 0)
    })
}

pub(crate) fn instruction(text: &str) -> bool {
    let text = text.trim_start();
    let text = [
        "First, ",
        "Next, ",
        "Then, ",
        "Finally, ",
        "You can ",
        "You should ",
        "You must ",
    ]
    .iter()
    .find_map(|prefix| {
        text.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &text[prefix.len()..])
    })
    .unwrap_or(text);
    starts(
        text,
        &[
            "to start,",
            "to begin,",
            "let's implement",
            "let us implement",
            "run",
            "install",
            "open",
            "create",
            "set",
            "add",
            "select",
            "click",
            "use",
            "start",
            "connect",
            "copy",
            "edit",
            "configure",
            "build",
            "test",
            "download",
            "enter",
            "execute",
            "save",
            "remove",
            "check",
            "verify",
            "restart",
            "ask the user",
            "安装",
            "运行",
            "打开",
            "创建",
            "设置",
            "添加",
            "选择",
            "点击",
            "执行",
            "复制",
            "编辑",
            "配置",
            "下载",
            "输入",
            "保存",
            "检查",
            "确认",
            "首先",
            "然后",
            "接下来",
            "最后",
            "请",
            "让我们通过实现",
            "まず",
            "次に",
            "最後に",
        ],
    ) || contains(text, &["してください", "しましょう"])
}

fn previous_paragraph<'a>(document: &'a Document, block: &Block) -> Option<&'a Block> {
    document.sections[block.section]
        .blocks
        .iter()
        .map(|&id| &document.blocks[id])
        .rfind(|candidate| {
            candidate.parent == block.parent
                && candidate.span.end <= block.span.start
                && !matches!(
                    candidate.kind,
                    BlockKind::HtmlComment | BlockKind::ThematicBreak
                )
        })
        .filter(|candidate| candidate.kind == BlockKind::Paragraph)
}

fn output_context(document: &Document, block: &Block) -> bool {
    let context = previous_paragraph(document, block)
        .map(|block| block_text(document, block))
        .unwrap_or_default();
    let heading = document.sections[block.section]
        .heading
        .as_deref()
        .unwrap_or_default();
    contains(
        &context,
        &[
            "following output",
            "following error",
            "output looks",
            "output should",
            "expected output",
            "will output",
            "will print",
            "get an error",
            "results in an error",
            "如下输出",
            "以下输出",
            "输出如下",
            "得到一个错误",
            "以下のエラー",
            "出力は",
            "エラーが発生",
            "エラーに",
        ],
    ) || starts(
        heading,
        &[
            "output",
            "expected output",
            "compiler output",
            "输出",
            "出力",
            "実行結果",
        ],
    )
}

fn code_body<'a>(document: &'a Document, block: &Block) -> &'a str {
    let raw = &document.source[block.span.start..block.span.end];
    let first = raw.lines().next().unwrap_or_default().trim_start();
    if first.starts_with("```") || first.starts_with("~~~") {
        let Some((_, body)) = raw.split_once('\n') else {
            return "";
        };
        let last = body.lines().next_back().unwrap_or_default().trim();
        if last.starts_with(&first[..3]) {
            return body.rsplit_once('\n').map_or("", |(body, _)| body);
        }
        body
    } else {
        raw
    }
}

pub(crate) fn runnable(document: &Document, block: &Block) -> bool {
    if block.kind != BlockKind::Code || output_context(document, block) {
        return false;
    }
    let language = block
        .code_language
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = code_body(document, block);
    let meaningful = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'));
    let Some(line) = meaningful else {
        return false;
    };
    if line.starts_with("{{") {
        return false;
    }
    if [
        "sh",
        "bash",
        "shell",
        "powershell",
        "pwsh",
        "cmd",
        "bat",
        "zsh",
        "fish",
    ]
    .contains(&language.as_str())
    {
        return true;
    }
    if language == "console" {
        return line.starts_with("$ ") || line.starts_with("> ") || line.starts_with("PS> ");
    }
    // Non-shell examples establish a task only when the adjacent prose tells
    // the reader to create/configure/run them; language tags alone are insufficient.
    previous_paragraph(document, block)
        .is_some_and(|previous| instruction(&block_text(document, previous)))
}

pub(crate) fn main_flow(document: &Document, block: &Block) -> bool {
    if is_quoted(document, block) || output_section(document, block.section) {
        return false;
    }
    match block.kind {
        BlockKind::Code => runnable(document, block),
        BlockKind::Paragraph => instruction(&block_text(document, block)),
        BlockKind::List if block.ordered == Some(true) => block
            .children
            .first()
            .map(|&id| &document.blocks[id])
            .and_then(|item| item.children.first())
            .map(|&id| &document.blocks[id])
            .is_some_and(|paragraph| {
                paragraph.kind == BlockKind::Paragraph
                    && instruction(&block_text(document, paragraph))
            }),
        _ => false,
    }
}

/// Opaque HTML may contain an earlier flow that the Markdown model cannot see.
pub(crate) fn opaque_flow(document: &Document) -> Option<Span> {
    document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Html)
        .find_map(|block| {
            let raw = &document.source[block.span.start..block.span.end];
            (raw.contains("```")
                || raw.contains("~~~")
                || raw
                    .split('<')
                    .skip(1)
                    .any(|tail| tail.starts_with(|ch: char| ch.is_ascii_uppercase())))
            .then_some(block.span)
        })
}

pub(crate) fn unclassified_example(document: &Document, block: &Block) -> bool {
    block.kind == BlockKind::Code
        && !is_quoted(document, block)
        && !output_section(document, block.section)
        && !output_context(document, block)
        && !runnable(document, block)
        && !matches!(
            block.code_language.as_deref(),
            Some("text" | "plaintext" | "console")
        )
        && !code_body(document, block).trim().is_empty()
}

pub(crate) fn rationale(text: &str, language: Language) -> bool {
    contains(
        text,
        match language {
            Language::En => &[
                "we chose",
                "we choose",
                "we decided",
                "we opted",
                "our decision",
                "the trade-off is",
                "the tradeoff is",
            ],
            Language::Zh => &[
                "我们选择",
                "我们决定",
                "之所以选择",
                "选择的原因",
                "选择了这个方案",
            ],
            Language::Ja => &["を選んだ理由", "を採用した理由", "を採用しました"],
        },
    )
}

fn normalized_heading(heading: &str) -> String {
    heading
        .split("{#")
        .next()
        .unwrap_or(heading)
        .trim()
        .trim_end_matches([':', '：', '?', '？'])
        .to_ascii_lowercase()
}

fn heading_role(heading: &str) -> Option<SectionType> {
    let text = normalized_heading(heading);
    for (role, names) in [
        (
            SectionType::Troubleshooting,
            &[
                "troubleshooting",
                "frequently asked questions",
                "faq",
                "exceptions",
                "known issues",
                "故障排查",
                "故障处理",
                "常见问题",
                "例外",
                "トラブルシューティング",
                "よくある質問",
            ][..],
        ),
        (
            SectionType::Rationale,
            &[
                "rationale",
                "design decisions",
                "trade-offs",
                "tradeoffs",
                "设计决策",
                "设计理由",
                "设计权衡",
                "設計判断",
                "設計の理由",
            ][..],
        ),
        (
            SectionType::Reference,
            &[
                "api reference",
                "configuration reference",
                "parameters",
                "return values",
                "options",
                "参数",
                "配置参考",
                "返回值",
                "パラメータ",
                "戻り値",
                "リファレンス",
                "オプション",
            ][..],
        ),
        (
            SectionType::Background,
            &[
                "background",
                "overview",
                "concepts",
                "description",
                "背景",
                "概述",
                "概要",
                "概念",
                "总览",
                "説明",
                "例外処理",
            ][..],
        ),
        (
            SectionType::Steps,
            &[
                "quick start",
                "quickstart",
                "getting started",
                "installation",
                "install",
                "procedure",
                "安装步骤",
                "快速开始",
                "操作步骤",
                "インストール",
                "クイックスタート",
                "手順",
            ][..],
        ),
    ] {
        if names.contains(&text.as_str()) {
            return Some(role);
        }
    }
    if starts(
        &text,
        &[
            "why we chose",
            "why we use",
            "why use",
            "为什么选择",
            "为什么要用",
            "採用した理由",
        ],
    ) {
        return Some(SectionType::Rationale);
    }
    if contains(
        &text,
        &[
            "よくある質問",
            "not working",
            "not formatting",
            "does not work",
        ],
    ) {
        return Some(SectionType::Troubleshooting);
    }
    if text.ends_with(" overview") {
        return Some(SectionType::Background);
    }
    None
}

pub(crate) fn decision_heading(heading: &str) -> bool {
    let heading = normalized_heading(heading);
    [
        "rationale",
        "design decisions",
        "设计决策",
        "设计理由",
        "设计权衡",
        "設計判断",
        "設計の理由",
    ]
    .contains(&heading.as_str())
        || starts(
            &heading,
            &[
                "why we chose",
                "why we choose",
                "why we use",
                "为什么选择",
                "採用した理由",
            ],
        )
}

fn reference_signal(document: &Document, block: &Block) -> bool {
    if block.kind == BlockKind::Table {
        return true;
    }
    if block.kind != BlockKind::Paragraph {
        return false;
    }
    let text = block_text(document, block);
    (starts(
        &text,
        &[
            "returns",
            "return type",
            "返回值",
            "返回类型",
            "函数会",
            "戻り値:",
            "引数の型:",
        ],
    ) || contains(&text, &["optional field", "must be annotated"]))
        && block.sentences.iter().any(|&id| {
            document.sentences[id]
                .fragments
                .iter()
                .any(|fragment| fragment.kind == FragmentKind::InlineCode)
        })
}

pub fn classify(document: &Document) -> Vec<SectionAnnotation> {
    document
        .sections
        .iter()
        .enumerate()
        .map(|(id, section)| {
            let content_end = section.children.first().map_or(section.span.end, |&child| {
                document.sections[child].span.start
            });
            let content_start = section
                .heading_span
                .map_or(section.span.start, |span| span.end);
            let mut annotation = SectionAnnotation {
                section: id,
                section_type: SectionType::Other,
                content_span: Span::new(content_start, content_end),
                evidence: Vec::new(),
            };
            let direct: Vec<_> = section
                .blocks
                .iter()
                .map(|&i| &document.blocks[i])
                .filter(|block| !is_quoted(document, block))
                .collect();
            let heading = direct
                .iter()
                .filter(|block| block.kind == BlockKind::Heading)
                .map(|block| block_text(document, block))
                .collect::<Vec<_>>()
                .join(" ");
            let role = heading_role(&heading);
            if [
                "knowledge check",
                "quiz",
                "exercise",
                "exercises",
                "知识检查",
                "练习",
                "知識チェック",
                "演習",
            ]
            .contains(&normalized_heading(&heading).as_str())
            {
                return annotation;
            }
            let content: Vec<_> = direct
                .iter()
                .copied()
                .filter(|block| block.kind != BlockKind::Heading)
                .collect();
            let navigation = content.iter().any(|block| block.kind == BlockKind::List)
                && content
                    .iter()
                    .filter(|block| block.kind == BlockKind::Paragraph)
                    .all(|block| {
                        !block.sentences.is_empty()
                            && block.sentences.iter().all(|&id| {
                                document.sentences[id]
                                    .fragments
                                    .iter()
                                    .any(|fragment| fragment.kind == FragmentKind::LinkText)
                                    || starts(
                                        &prose_text(&document.sentences[id]),
                                        &["read the", "see the", "looking for"],
                                    )
                            })
                    });
            let api_heading = section.heading.as_deref().is_some_and(|heading| {
                let heading = normalized_heading(heading);
                heading.ends_with("()") && !heading.contains(' ')
            });
            let choice = content.iter().copied().find(|block| {
                block.kind == BlockKind::Paragraph
                    && block.sentences.iter().any(|&i| {
                        let sentence = &document.sentences[i];
                        rationale(&prose_text(sentence), sentence.language)
                    })
            });
            let signal = if matches!(
                role,
                Some(SectionType::Troubleshooting | SectionType::Rationale)
            ) {
                Some((
                    role.unwrap(),
                    "heading",
                    section.heading_span.unwrap_or(section.span),
                ))
            } else if api_heading || role == Some(SectionType::Reference) {
                Some((
                    SectionType::Reference,
                    "reference_heading",
                    section.heading_span.unwrap_or(section.span),
                ))
            } else if let Some(block) = content
                .iter()
                .find(|block| reference_signal(document, block))
            {
                Some((SectionType::Reference, "reference_content", block.span))
            } else if role == Some(SectionType::Background) && !navigation {
                Some((
                    SectionType::Background,
                    "heading",
                    section.heading_span.unwrap_or(section.span),
                ))
            } else if let Some(block) = content.iter().find(|block| main_flow(document, block)) {
                Some((
                    SectionType::Steps,
                    if block.kind == BlockKind::Code {
                        "runnable_example"
                    } else {
                        "instruction"
                    },
                    block.span,
                ))
            } else if role == Some(SectionType::Steps) {
                Some((
                    SectionType::Steps,
                    "heading",
                    section.heading_span.unwrap_or(section.span),
                ))
            } else if let Some(block) = choice {
                Some((SectionType::Rationale, "decision_prose", block.span))
            } else if !navigation {
                let prose: Vec<_> = content
                    .iter()
                    .filter(|block| block.kind == BlockKind::Paragraph)
                    .collect();
                let chars: usize = prose
                    .iter()
                    .map(|block| block_text(document, block).chars().count())
                    .sum();
                (prose.len() >= 2 && chars >= 160)
                    .then(|| (SectionType::Background, "explanatory_prose", prose[0].span))
            } else {
                None
            };
            if let Some((role, signal, span)) = signal {
                annotation.section_type = role;
                annotation.evidence.push(SectionEvidence { signal, span });
            }
            annotation
        })
        .collect()
}
