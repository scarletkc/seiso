use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::diagnostics::Span;
use markdown::mdast::Node;
use regex::Regex;
use serde_yaml_ng::Value;

use crate::md::{mapping, *};

static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bat line (\d+)").unwrap());

pub fn parse(source: &str) -> Result<Document, ParseError> {
    parse_with_options(source, ParseOptions::default())
}

pub fn parse_with_options(source: &str, options: ParseOptions) -> Result<Document, ParseError> {
    let mut markdown_options = match options.flavor {
        MarkdownFlavor::Gfm => markdown::ParseOptions::gfm(),
        MarkdownFlavor::CommonMark => markdown::ParseOptions::default(),
    };
    // markdown-rs enables TOML and YAML together; only YAML is metadata here.
    markdown_options.constructs.frontmatter = source
        .lines()
        .next()
        .is_some_and(|line| line.trim_start_matches('\u{feff}').trim_end() == "---");
    let tree = markdown::to_mdast(source, &markdown_options).map_err(|error| ParseError {
        message: error.to_string(),
    })?;
    let mut builder = DocumentBuilder::new(source);
    builder.read_definitions(&tree);
    builder.walk_blocks(&tree);
    builder.detect_language();
    Ok(builder.document)
}

struct DocumentBuilder<'a> {
    source: &'a str,
    document: Document,
    definitions: BTreeMap<String, Definition>,
    declared_language: Option<Language>,
    section_stack: Vec<usize>,
    section_floor: usize,
    comment_scanner: CommentScanner,
}

impl<'a> DocumentBuilder<'a> {
    fn new(source: &'a str) -> Self {
        let document = Document {
            source: source.to_owned(),
            frontmatter: None,
            sections: vec![Section {
                parent: None,
                children: Vec::new(),
                depth: 0,
                heading: None,
                heading_span: None,
                span: Span {
                    start: 0,
                    end: source.len(),
                },
                blocks: Vec::new(),
            }],
            blocks: Vec::new(),
            sentences: Vec::new(),
            links: Vec::new(),
            comments: Vec::new(),
            identifiers: Vec::new(),
            language: Language::En,
        };
        Self {
            source,
            document,
            definitions: BTreeMap::new(),
            declared_language: None,
            section_stack: vec![0],
            section_floor: 1,
            comment_scanner: CommentScanner::default(),
        }
    }

    fn read_definitions(&mut self, tree: &Node) {
        let mut stack = vec![tree];
        while let Some(node) = stack.pop() {
            if let Node::Definition(definition) = node {
                self.definitions
                    .entry(normalize_reference(&definition.identifier))
                    .or_insert_with(|| Definition {
                        destination: definition.url.clone(),
                        title: definition.title.clone(),
                        span: destination_span(self.source, node, true),
                    });
            }
            if let Node::Yaml(yaml) = node {
                let span = node_span(node);
                // YAML locations count from the line after the opening fence.
                let line_offset = self.source[..span.start].matches('\n').count() + 1;
                self.document.frontmatter = Some(frontmatter(&yaml.value, span, line_offset));
            }
            if let Some(children) = node.children() {
                stack.extend(children.iter().rev());
            }
        }
        if self.document.frontmatter.is_none() && looks_like_unclosed_frontmatter(self.source) {
            let span = Span {
                start: 0,
                end: self.source.len(),
            };
            self.document.frontmatter = Some(Frontmatter {
                span,
                raw: self.source.lines().skip(1).collect::<Vec<_>>().join("\n"),
                kind: None,
                lang: None,
                canonical: None,
                errors: vec![FrontmatterError {
                    span,
                    message: "YAML frontmatter has no closing fence".to_owned(),
                }],
            });
        }
        self.declared_language = self
            .document
            .frontmatter
            .as_ref()
            .and_then(|frontmatter| frontmatter.lang.as_deref())
            .and_then(Language::from_name);
    }

    fn walk_blocks(&mut self, tree: &Node) {
        let mut pending = vec![Walk::Node(tree, None)];
        while let Some(event) = pending.pop() {
            let (node, parent) = match event {
                Walk::Node(node, parent) => (node, parent),
                Walk::Exit {
                    sections,
                    floor,
                    end,
                } => {
                    for section in &self.section_stack[sections.len()..] {
                        self.document.sections[*section].span.end = end;
                    }
                    self.section_stack = sections;
                    self.section_floor = floor;
                    continue;
                }
            };
            self.add_section(node);
            let Some(kind) = block_kind(node) else {
                if let Some(children) = node.children() {
                    pending.extend(children.iter().rev().map(|child| Walk::Node(child, parent)));
                }
                continue;
            };
            let block = self.add_block(node, parent, kind);
            if matches!(
                kind,
                BlockKind::Paragraph | BlockKind::Heading | BlockKind::TableCell
            ) {
                self.add_inline(node, block);
            } else if matches!(kind, BlockKind::Html | BlockKind::HtmlComment) {
                self.comment_scanner.extract(
                    self.source,
                    node_span(node),
                    &mut self.document.comments,
                );
            } else if let Some(children) = node.children() {
                if matches!(
                    kind,
                    BlockKind::Blockquote | BlockKind::ListItem | BlockKind::FootnoteDefinition
                ) {
                    pending.push(Walk::Exit {
                        sections: self.section_stack.clone(),
                        floor: self.section_floor,
                        end: node_span(node).end,
                    });
                    self.section_floor = self.section_stack.len();
                }
                pending.extend(
                    children
                        .iter()
                        .rev()
                        .map(|child| Walk::Node(child, Some(block))),
                );
            }
        }
    }

    fn add_section(&mut self, node: &Node) {
        if let Node::Heading(heading) = node {
            let span = node_span(node);
            while self.section_stack.len() > self.section_floor
                && self.document.sections[*self.section_stack.last().unwrap_or(&0)].depth
                    >= heading.depth
            {
                if let Some(section) = self.section_stack.pop() {
                    self.document.sections[section].span.end = span.start;
                }
            }
            let parent_section = *self.section_stack.last().unwrap_or(&0);
            let section = self.document.sections.len();
            self.document.sections[parent_section]
                .children
                .push(section);
            self.document.sections.push(Section {
                parent: Some(parent_section),
                children: Vec::new(),
                depth: heading.depth,
                heading: Some(node.to_string()),
                heading_span: Some(span),
                span: Span {
                    start: span.start,
                    end: self.source.len(),
                },
                blocks: Vec::new(),
            });
            self.section_stack.push(section);
        }
    }

    fn add_block(&mut self, node: &Node, parent: Option<usize>, kind: BlockKind) -> usize {
        let section = *self.section_stack.last().unwrap_or(&0);
        let block = self.document.blocks.len();
        self.document.blocks.push(Block {
            kind,
            span: node_span(node),
            parent,
            children: Vec::new(),
            sentences: Vec::new(),
            section,
            ordered: if let Node::List(list) = node {
                Some(list.ordered)
            } else {
                None
            },
            checked: if let Node::ListItem(item) = node {
                item.checked
            } else {
                None
            },
            code_language: if let Node::Code(code) = node {
                code.lang.clone()
            } else {
                None
            },
        });
        self.document.sections[section].blocks.push(block);
        if let Some(parent) = parent {
            self.document.blocks[parent].children.push(block);
        }
        block
    }

    fn add_inline(&mut self, node: &Node, block: usize) {
        let kind = self.document.blocks[block].kind;
        let section = self.document.blocks[block].section;
        let fragments = inline(
            self.source,
            node,
            &self.definitions,
            &mut self.document,
            &mut self.comment_scanner,
        );
        if kind == BlockKind::Heading {
            self.document.sections[section].heading = Some(
                fragments
                    .iter()
                    .filter(|fragment| fragment.kind != FragmentKind::LinkDestination)
                    .map(|fragment| fragment.text.as_str())
                    .collect(),
            );
        }
        for fragment in &fragments {
            if fragment.kind == FragmentKind::InlineCode && is_identifier(&fragment.text) {
                self.document.identifiers.push(Identifier {
                    text: fragment.text.clone(),
                    span: fragment.span,
                    block,
                    section,
                });
            }
        }
        for sentence in sentences(fragments, block, self.declared_language) {
            self.document.blocks[block]
                .sentences
                .push(self.document.sentences.len());
            self.document.sentences.push(sentence);
        }
    }

    fn detect_language(&mut self) {
        let prose = self
            .document
            .sentences
            .iter()
            .flat_map(|sentence| &sentence.fragments)
            .filter(|fragment| is_language_evidence(fragment.kind))
            .map(|fragment| fragment.text.as_str())
            .collect::<String>();
        self.document.language = self
            .declared_language
            .unwrap_or_else(|| detect_language(&prose));
    }
}

enum Walk<'a> {
    Node(&'a Node, Option<usize>),
    Exit {
        sections: Vec<usize>,
        floor: usize,
        end: usize,
    },
}

fn node_span(node: &Node) -> Span {
    node.position()
        .map(|position| Span {
            start: position.start.offset,
            end: position.end.offset,
        })
        .unwrap_or(Span { start: 0, end: 0 })
}

fn looks_like_unclosed_frontmatter(source: &str) -> bool {
    let mut lines = source.lines();
    if lines.next().is_none_or(|line| line.trim_end() != "---") {
        return false;
    }
    let Some(line) = lines.find(|line| !line.trim().is_empty()) else {
        return false;
    };
    let Some((key, _)) = line.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn block_kind(node: &Node) -> Option<BlockKind> {
    Some(match node {
        Node::Heading(_) => BlockKind::Heading,
        Node::Paragraph(_) => BlockKind::Paragraph,
        Node::List(_) => BlockKind::List,
        Node::ListItem(_) => BlockKind::ListItem,
        Node::Table(_) => BlockKind::Table,
        Node::TableRow(_) => BlockKind::TableRow,
        Node::TableCell(_) => BlockKind::TableCell,
        Node::Code(_) => BlockKind::Code,
        Node::Blockquote(_) => BlockKind::Blockquote,
        Node::Html(html) if html.value.trim().starts_with("<!--") => BlockKind::HtmlComment,
        Node::Html(_) => BlockKind::Html,
        Node::ThematicBreak(_) => BlockKind::ThematicBreak,
        Node::Definition(_) => BlockKind::Definition,
        Node::FootnoteDefinition(_) => BlockKind::FootnoteDefinition,
        _ => return None,
    })
}

fn frontmatter(raw: &str, span: Span, line_offset: usize) -> Frontmatter {
    let mut result = Frontmatter {
        span,
        raw: raw.to_owned(),
        kind: None,
        lang: None,
        canonical: None,
        errors: Vec::new(),
    };
    let parsed = match serde_yaml_ng::from_str::<Value>(raw) {
        Ok(Value::Null) => return result,
        Ok(Value::Mapping(mapping)) => mapping,
        Ok(_) => {
            result.errors.push(FrontmatterError {
                span,
                message: "YAML frontmatter must be a mapping".to_owned(),
            });
            return result;
        }
        Err(error) => {
            result.errors.push(FrontmatterError {
                span,
                message: file_line_numbers(&error.to_string(), line_offset),
            });
            return result;
        }
    };
    for (key, target) in [("kind", &mut result.kind), ("lang", &mut result.lang)] {
        if let Some(value) = parsed.get(Value::String(key.to_owned())) {
            if let Some(value) = value.as_str() {
                *target = Some(value.to_owned());
            } else {
                result.errors.push(FrontmatterError {
                    span,
                    message: format!("frontmatter `{key}` must be a string"),
                });
            }
        }
    }
    if let Some(value) = parsed.get(Value::String("canonical".to_owned())) {
        if let Some(value) = value.as_bool() {
            result.canonical = Some(value);
        } else {
            result.errors.push(FrontmatterError {
                span,
                message: "frontmatter `canonical` must be a boolean".to_owned(),
            });
        }
    }
    result
}

/// Report YAML locations as lines of the Markdown file.
fn file_line_numbers(message: &str, offset: usize) -> String {
    LINE.replace_all(message, |captures: &regex::Captures<'_>| {
        captures[1].parse::<usize>().map_or_else(
            |_| captures[0].to_owned(),
            |line| format!("at line {}", line + offset),
        )
    })
    .into_owned()
}

struct Definition {
    destination: String,
    title: Option<String>,
    span: Span,
}

fn normalize_reference(identifier: &str) -> String {
    identifier
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .to_uppercase()
}

enum InlineEvent<'a> {
    Node(&'a Node, FragmentKind),
    Destination(Fragment),
}

fn inline(
    source: &str,
    root: &Node,
    definitions: &BTreeMap<String, Definition>,
    document: &mut Document,
    comment_scanner: &mut CommentScanner,
) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    let mut pending = vec![InlineEvent::Node(root, FragmentKind::Text)];
    while let Some(event) = pending.pop() {
        let (node, context) = match event {
            InlineEvent::Node(node, context) => (node, context),
            InlineEvent::Destination(fragment) => {
                fragments.push(fragment);
                continue;
            }
        };
        let span = node_span(node);
        match node {
            Node::Text(text) => {
                fragments.push(mapping::fragment(source, context, &text.value, span))
            }
            Node::InlineCode(code) => {
                let value = code.value.replace("\r\n", " ").replace(['\r', '\n'], " ");
                fragments.push(mapping::fragment(
                    source,
                    FragmentKind::InlineCode,
                    &value,
                    span,
                ));
            }
            Node::Break(_) => fragments.push(mapping::fragment(source, context, "\n", span)),
            Node::Html(_) => comment_scanner.extract(source, span, &mut document.comments),
            Node::Link(_) | Node::Image(_) | Node::LinkReference(_) | Node::ImageReference(_) => {
                let link = inline_link(source, node, definitions);
                if let Some(link) = link {
                    pending.push(InlineEvent::Destination(mapping::fragment(
                        source,
                        FragmentKind::LinkDestination,
                        &link.destination,
                        link.destination_span,
                    )));
                    document.links.push(link);
                }
                inline_label(source, node, &mut pending, &mut fragments);
            }
            _ => {
                if let Some(children) = node.children() {
                    pending.extend(
                        children
                            .iter()
                            .rev()
                            .map(|child| InlineEvent::Node(child, context)),
                    );
                }
            }
        }
    }
    fragments
}

fn inline_label<'a>(
    source: &str,
    node: &'a Node,
    pending: &mut Vec<InlineEvent<'a>>,
    fragments: &mut Vec<Fragment>,
) {
    let span = node_span(node);
    if let Some(children) = node.children() {
        if children.is_empty() {
            fragments.push(mapping::fragment(source, FragmentKind::LinkText, "", span));
        }
        pending.extend(
            children
                .iter()
                .rev()
                .map(|child| InlineEvent::Node(child, FragmentKind::LinkText)),
        );
    } else {
        let alt = match node {
            Node::Image(image) => &image.alt,
            Node::ImageReference(image) => &image.alt,
            _ => "",
        };
        fragments.push(mapping::fragment(source, FragmentKind::LinkText, alt, span));
    }
}

fn reference_link(
    identifier: &str,
    span: Span,
    image: bool,
    definitions: &BTreeMap<String, Definition>,
) -> Option<RawLink> {
    definitions
        .get(&normalize_reference(identifier))
        .map(|definition| RawLink {
            span,
            destination: definition.destination.clone(),
            destination_span: definition.span,
            title: definition.title.clone(),
            reference: Some(identifier.to_owned()),
            image,
        })
}

fn inline_link(
    source: &str,
    node: &Node,
    definitions: &BTreeMap<String, Definition>,
) -> Option<RawLink> {
    let span = node_span(node);
    match node {
        Node::Link(link) => Some(RawLink {
            span,
            destination: link.url.clone(),
            destination_span: destination_span(source, node, false),
            title: link.title.clone(),
            reference: None,
            image: false,
        }),
        Node::Image(link) => Some(RawLink {
            span,
            destination: link.url.clone(),
            destination_span: destination_span(source, node, false),
            title: link.title.clone(),
            reference: None,
            image: true,
        }),
        Node::LinkReference(link) => reference_link(&link.identifier, span, false, definitions),
        Node::ImageReference(link) => reference_link(&link.identifier, span, true, definitions),
        _ => None,
    }
}

fn destination_span(source: &str, node: &Node, definition: bool) -> Span {
    let span = node_span(node);
    let raw = &source[span.start..span.end];
    if raw.starts_with('<') && raw.ends_with('>') {
        return Span {
            start: span.start + 1,
            end: span.end - 1,
        };
    }
    let Some(delimiter) = closing_label(raw) else {
        return span;
    };
    let needle = if definition { "]:" } else { "](" };
    if !raw[delimiter..].starts_with(needle) {
        return span;
    }
    let rest = &raw[delimiter + 2..];
    let whitespace = rest.len() - rest.trim_start().len();
    let start = delimiter + 2 + whitespace;
    let angle = raw[start..].starts_with('<');
    let start = start + usize::from(angle);
    let mut end = start;
    let mut depth = 0usize;
    let mut escaped = false;
    for (offset, ch) in raw[start..].char_indices() {
        if !escaped
            && ((angle && ch == '>')
                || (!angle && (ch.is_whitespace() || (ch == ')' && depth == 0))))
        {
            break;
        }
        if !escaped && !angle {
            if ch == '(' {
                depth += 1;
            }
            if ch == ')' {
                depth = depth.saturating_sub(1);
            }
        }
        end = start + offset + ch.len_utf8();
        escaped = !escaped && ch == '\\';
    }
    Span {
        start: span.start + start,
        end: span.start + end,
    }
}

fn closing_label(raw: &str) -> Option<usize> {
    let opening = usize::from(raw.starts_with('!'));
    if !raw[opening..].starts_with('[') {
        return None;
    }
    let mut depth = 0usize;
    let mut escaped = false;
    let mut code_fence = 0usize;
    let mut cursor = opening;
    while cursor < raw.len() {
        let ch = raw[cursor..].chars().next()?;
        if !escaped && ch == '`' {
            let ticks = raw[cursor..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            if code_fence == 0 {
                code_fence = ticks;
            } else if ticks == code_fence {
                code_fence = 0;
            }
            cursor += ticks;
            continue;
        }
        if !escaped && code_fence == 0 {
            if ch == '[' {
                depth += 1;
            }
            if ch == ']' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(cursor);
                }
            }
        }
        escaped = !escaped && ch == '\\';
        cursor += ch.len_utf8();
    }
    None
}

#[derive(Default)]
struct CommentScanner {
    raw_tag: Option<String>,
}

impl CommentScanner {
    fn extract(&mut self, source: &str, span: Span, comments: &mut Vec<HtmlComment>) {
        let raw = &source[span.start..span.end];
        let lower = raw.to_ascii_lowercase();
        let mut cursor = 0;
        while cursor < raw.len() {
            if let Some(tag) = &self.raw_tag {
                let closing = format!("</{tag}");
                let Some(start) = lower[cursor..]
                    .match_indices(&closing)
                    .find_map(|(index, _)| {
                        let start = cursor + index;
                        lower[start + closing.len()..]
                            .chars()
                            .next()
                            .is_some_and(|ch| ch == '>' || ch.is_ascii_whitespace())
                            .then_some(start)
                    })
                else {
                    return;
                };
                cursor = start;
                self.raw_tag = None;
            }
            let Some(start) = raw[cursor..].find('<').map(|start| cursor + start) else {
                break;
            };
            if raw[start..].starts_with("<!--") {
                let Some(end) = raw[start + 4..].find("-->").map(|end| start + 4 + end) else {
                    break;
                };
                comments.push(HtmlComment {
                    span: Span {
                        start: span.start + start,
                        end: span.start + end + 3,
                    },
                    content: raw[start + 4..end].to_owned(),
                });
                cursor = end + 3;
                continue;
            }
            if raw[start..].starts_with("<![CDATA[") {
                let Some(end) = raw[start + 9..].find("]]>") else {
                    break;
                };
                cursor = start + 9 + end + 3;
                continue;
            }
            let tag: String = lower[start + 1..]
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let mut quote = None;
            let mut end = None;
            for (offset, ch) in raw[start + 1..].char_indices() {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    if matches!(ch, '\'' | '"') {
                        quote = Some(ch);
                    } else if ch == '>' {
                        end = Some(start + 1 + offset + 1);
                        break;
                    }
                }
            }
            let Some(end) = end else {
                break;
            };
            if [
                "script",
                "style",
                "textarea",
                "title",
                "xmp",
                "iframe",
                "noembed",
                "noframes",
                "plaintext",
            ]
            .contains(&tag.as_str())
            {
                self.raw_tag = Some(tag);
            }
            cursor = end;
        }
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().any(char::is_alphanumeric)
        && value
            .chars()
            .all(|ch| ch.is_alphanumeric() || "_./:-$[]<>".contains(ch))
}

/// Inline code and link destinations name identifiers, not the language of the prose.
fn is_language_evidence(kind: FragmentKind) -> bool {
    matches!(kind, FragmentKind::Text | FragmentKind::LinkText)
}

/// Compare CJK characters with Latin words, so English terms in Chinese or
/// Japanese prose do not outweigh the sentence around them.
fn detect_language(text: &str) -> Language {
    let mut latin_words = 0;
    let mut in_word = false;
    let mut han = 0;
    let mut kana = 0;
    for ch in text.chars() {
        let latin = ch.is_ascii_alphabetic();
        if latin && !in_word {
            latin_words += 1;
        }
        in_word = latin;
        match ch {
            '\u{3040}'..='\u{30ff}' | '\u{ff66}'..='\u{ff9f}' => kana += 1,
            '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{20000}'..='\u{323af}' => {
                han += 1
            }
            _ => {}
        }
    }
    if han + kana > latin_words {
        if kana > 0 { Language::Ja } else { Language::Zh }
    } else {
        Language::En
    }
}

fn sentences(fragments: Vec<Fragment>, block: usize, language: Option<Language>) -> Vec<Sentence> {
    let mut prose = String::new();
    let mut positions = Vec::with_capacity(fragments.len());
    for fragment in &fragments {
        let start = prose.len();
        if fragment.kind != FragmentKind::LinkDestination {
            prose.push_str(&fragment.text);
        }
        positions.push(Span {
            start,
            end: prose.len(),
        });
    }
    if prose.trim().is_empty() {
        if !fragments
            .iter()
            .any(|fragment| fragment.kind == FragmentKind::LinkDestination)
        {
            return Vec::new();
        }
        let span = fragments
            .iter()
            .find(|fragment| fragment.kind == FragmentKind::LinkText)
            .or_else(|| fragments.first())
            .map(|fragment| fragment.span)
            .unwrap_or(Span { start: 0, end: 0 });
        return vec![Sentence {
            span,
            block,
            language: language.unwrap_or_default(),
            fragments,
        }];
    }
    let mut boundaries = vec![0];
    for (fragment, position) in fragments.iter().zip(&positions) {
        if matches!(
            fragment.kind,
            FragmentKind::InlineCode | FragmentKind::LinkDestination
        ) {
            continue;
        }
        for (offset, ch) in fragment.text.char_indices() {
            let end = position.start + offset + ch.len_utf8();
            let next = prose[end..].chars().next();
            if matches!(ch, '。' | '！' | '？')
                || (matches!(ch, '.' | '!' | '?') && next.is_none_or(char::is_whitespace))
            {
                boundaries.push(end);
            }
        }
    }
    if boundaries.last().copied() != Some(prose.len()) {
        boundaries.push(prose.len());
    }
    let mut result = Vec::new();
    for pair in boundaries.windows(2) {
        let text = &prose[pair[0]..pair[1]];
        if text.trim().is_empty() {
            continue;
        }
        let start = pair[0] + text.len() - text.trim_start().len();
        let end = pair[1] - (text.len() - text.trim_end().len());
        let mut parts = Vec::new();
        for (fragment, position) in fragments.iter().zip(&positions) {
            if fragment.kind == FragmentKind::LinkDestination {
                if (position.start > pair[0] || position.start == 0) && position.start <= pair[1] {
                    parts.push(fragment.clone());
                }
            } else {
                let left = start.max(position.start);
                let right = end.min(position.end);
                if left < right {
                    if left == position.start && right == position.end {
                        parts.push(fragment.clone());
                    } else {
                        parts.push(mapping::slice_fragment(
                            fragment,
                            left - position.start,
                            right - position.start,
                        ));
                    }
                }
            }
        }
        let Some(first) = parts
            .iter()
            .find(|fragment| fragment.kind != FragmentKind::LinkDestination)
        else {
            continue;
        };
        let span = parts
            .iter()
            .filter(|fragment| fragment.kind != FragmentKind::LinkDestination)
            .fold(first.span, |span, fragment| Span {
                start: span.start.min(fragment.span.start),
                end: span.end.max(fragment.span.end),
            });
        result.push(Sentence {
            span,
            block,
            language: language.unwrap_or_else(|| {
                detect_language(
                    &parts
                        .iter()
                        .filter(|fragment| is_language_evidence(fragment.kind))
                        .map(|fragment| fragment.text.as_str())
                        .collect::<String>(),
                )
            }),
            fragments: parts,
        });
    }
    result
}
