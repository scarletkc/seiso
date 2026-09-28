use seiso::diagnostics::{SourceMap, Span};
use seiso::md::{
    BlockKind, Document, FragmentKind, Language, MarkdownFlavor, ParseOptions, parse,
    parse_with_options,
};

fn fragments(document: &Document) -> impl Iterator<Item = &seiso::md::Fragment> {
    document
        .sentences
        .iter()
        .flat_map(|sentence| &sentence.fragments)
}

#[test]
fn heading_facts_keep_rendered_spacing_and_alt_text_without_html_or_destinations() {
    let source = "# Hello!  *World*. <span id=custom>Value</span> ![icon](icon.png) &amp; `code`\n";
    let document = parse(source).unwrap();
    assert_eq!(
        document.sections[1].heading.as_deref(),
        Some("Hello!  World. Value icon & code")
    );
    assert_eq!(
        document.sections[1].heading_span,
        Some(Span::new(0, source.len() - 1))
    );
    assert!(
        document
            .links
            .iter()
            .any(|link| link.destination == "icon.png" && link.image)
    );
    assert_source_ranges(&document);
    let code = fragments(&document)
        .find(|fragment| fragment.kind == FragmentKind::InlineCode)
        .unwrap();
    assert_eq!(&source[code.span.start..code.span.end], "`code`");
    let escaped = parse("# \\<span> and `a < b`\n").unwrap();
    assert_eq!(
        escaped.sections[1].heading.as_deref(),
        Some("<span> and a < b")
    );
}

fn assert_source_ranges(document: &Document) {
    let valid = |span: Span| {
        assert!(
            span.start <= span.end && span.end <= document.source.len(),
            "{span:?}"
        );
        assert!(document.source.is_char_boundary(span.start), "{span:?}");
        assert!(document.source.is_char_boundary(span.end), "{span:?}");
    };
    for block in &document.blocks {
        valid(block.span);
    }
    for section in &document.sections {
        valid(section.span);
    }
    for sentence in &document.sentences {
        valid(sentence.span);
    }
    for link in &document.links {
        valid(link.span);
        valid(link.destination_span);
    }
    for comment in &document.comments {
        valid(comment.span);
    }
    for fragment in fragments(document) {
        valid(fragment.span);
        let mut covered = 0;
        for segment in &fragment.mapping {
            valid(segment.source);
            assert_eq!(segment.text.start, covered);
            assert!(fragment.text.is_char_boundary(segment.text.start));
            assert!(fragment.text.is_char_boundary(segment.text.end));
            assert!(
                segment.text.start < segment.text.end && segment.text.end <= fragment.text.len()
            );
            covered = segment.text.end;
        }
        assert_eq!(covered, fragment.text.len(), "{fragment:?}");
    }
}

#[test]
fn byte_mapping_survives_entities_escapes_unicode_and_crlf() {
    let source = "# 标题\r\n\r\n目前 \\*使用 &amp; &#x65E5; `v1.2.3`。次の文です！ English. Next.";
    let document = parse(source).unwrap();
    assert_source_ranges(&document);
    let text = fragments(&document)
        .find(|fragment| fragment.text.contains("*使用"))
        .unwrap();
    for (decoded, raw) in [("*", "\\*"), ("&", "&amp;"), ("日", "&#x65E5;")] {
        let start = text.text.find(decoded).unwrap();
        let span = text
            .source_span(Span {
                start,
                end: start + decoded.len(),
            })
            .unwrap();
        assert_eq!(&source[span.start..span.end], raw);
    }
    let code = fragments(&document)
        .find(|fragment| fragment.kind == FragmentKind::InlineCode)
        .unwrap();
    let span = code.source_span(Span { start: 0, end: 1 }).unwrap();
    assert_eq!(&source[span.start..span.end], "v");
    let location = SourceMap::new(source).location(span.start);
    assert_eq!((location.row, location.column), (3, 25));
    assert_eq!(document.sentences.len(), 5);
    assert!(
        document
            .sentences
            .iter()
            .any(|sentence| sentence.language == Language::Zh)
    );
    assert!(
        document
            .sentences
            .iter()
            .any(|sentence| sentence.language == Language::Ja)
    );
}

#[test]
fn language_detection_counts_latin_words_and_ignores_code_and_destinations() {
    for (source, expected) in [
        (
            "目前版本是 v1.2.3，配置见 `docs/reference/configuration.md` 与 `src/config/mod.rs`。",
            Language::Zh,
        ),
        (
            "根据你的要求，我已经把 `DupSettings.min_identifiers` 改成 3。",
            Language::Zh,
        ),
        ("运行 seiso check 检查 Markdown 文档。", Language::Zh),
        (
            "設定は `seiso.toml` の include で指定します。",
            Language::Ja,
        ),
        ("See [the guide](指南/安装.md) for setup.", Language::En),
        ("Set `名前` to the display name.", Language::En),
        ("Thanks to 张三 for the fix.", Language::En),
    ] {
        let document = parse(source).unwrap();
        assert_eq!(document.language, expected, "{source}");
        assert!(
            document
                .sentences
                .iter()
                .all(|sentence| sentence.language == expected),
            "{source}"
        );
    }
}

#[test]
fn section_hierarchy_and_ranges_close_at_sibling_headings() {
    let source = "Preamble\n\n# A\n\nA text\n\n### Deep\n\nDeep text\n\n## B\n\n# C\n";
    let document = parse(source).unwrap();
    assert_eq!(document.sections.len(), 5);
    assert_eq!(document.sections[0].children, vec![1, 4]);
    assert_eq!(document.sections[1].children, vec![2, 3]);
    assert_eq!(document.sections[2].span.end, source.find("## B").unwrap());
    assert_eq!(document.sections[1].span.end, source.find("# C").unwrap());
    assert_eq!(document.sections[4].span.end, source.len());
    assert_eq!(document.blocks[0].section, 0);
}

#[test]
fn gfm_tables_tasks_strikethrough_and_autolinks_are_content_facts() {
    let source = "- [x] Done\n- [ ] Next\n\n| Option | Value |\n| --- | --- |\n| `key` | ~~old~~ |\n\nhttps://example.com\n";
    let document = parse(source).unwrap();
    assert_eq!(
        document
            .blocks
            .iter()
            .filter_map(|block| block.checked)
            .collect::<Vec<_>>(),
        vec![true, false]
    );
    assert_eq!(
        document
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::TableCell)
            .count(),
        4
    );
    assert_eq!(document.links[0].destination, "https://example.com");
    assert_eq!(document.identifiers[0].text, "key");
    assert!(fragments(&document).any(|fragment| fragment.text == "old"));
    assert_source_ranges(&document);
    let commonmark = parse_with_options(
        source,
        ParseOptions {
            flavor: MarkdownFlavor::CommonMark,
        },
    )
    .unwrap();
    assert!(
        !commonmark
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Table)
    );
    assert!(commonmark.links.is_empty());
}

#[test]
fn references_resolve_case_and_whitespace_and_keep_definition_ranges() {
    let source = "See [the docs][A  B], [a b] and [missing].\n\n[a b]: <docs/a&amp;b.md> \"Manual\"\n[a b]: wrong.md\n";
    let document = parse(source).unwrap();
    assert_eq!(document.links.len(), 2, "{document:#?}");
    for link in &document.links {
        assert_eq!(link.destination, "docs/a&b.md");
        assert_eq!(link.title.as_deref(), Some("Manual"));
        assert_eq!(
            &source[link.destination_span.start..link.destination_span.end],
            "docs/a&amp;b.md"
        );
    }
    assert!(
        fragments(&document).any(|fragment| fragment.kind == FragmentKind::LinkDestination
            && fragment.text == "docs/a&b.md")
    );
    assert!(fragments(&document).any(|fragment| fragment.text.contains("[missing]")));
    assert_source_ranges(&document);
}

#[test]
fn link_destinations_keep_parentheses_entities_and_escaped_spaces() {
    let source = "[link](docs/a(b).md) [angle](<文档/a b.md>) ![image](img.png) <a@example.com>";
    let document = parse(source).unwrap();
    let destinations = document
        .links
        .iter()
        .map(|link| source[link.destination_span.start..link.destination_span.end].to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        destinations,
        ["docs/a(b).md", "文档/a b.md", "img.png", "a@example.com"]
    );
    assert!(document.links[2].image);
    assert_eq!(document.links[3].destination, "mailto:a@example.com");
    assert_source_ranges(&document);
}

#[test]
fn comments_only_come_from_html_and_not_code_examples() {
    let source = "<!-- seiso: allow-file VOX001 -- historical -->\n\nInline <!-- note --> content.\n\n`<!-- not a comment -->`\n\n```md\n<!-- seiso: allow VOX001 -- example -->\n```\n\n    <!-- indented code -->\n\n<script>let x = '<!-- not a comment -->';</script>\n";
    let document = parse(source).unwrap();
    assert_eq!(document.comments.len(), 2);
    assert_eq!(
        document.comments[0].content,
        " seiso: allow-file VOX001 -- historical "
    );
    assert_eq!(document.comments[1].content, " note ");
    assert_source_ranges(&document);
}

#[test]
fn frontmatter_is_typed_without_resolving_kind_or_hiding_failures() {
    let valid =
        parse("---\nkind: howto\nlang: ja\ncanonical: true\ncustom: value\n---\nText.").unwrap();
    let frontmatter = valid.frontmatter.unwrap();
    assert_eq!(frontmatter.kind.as_deref(), Some("howto"));
    assert_eq!(frontmatter.lang.as_deref(), Some("ja"));
    assert_eq!(frontmatter.canonical, Some(true));
    assert!(frontmatter.errors.is_empty());
    assert_eq!(valid.language, Language::Ja);
    for source in [
        "---\nkind: [\n---\nBody",
        "---\n- value\n---\nBody",
        "---\nkind: true\nlang: 3\ncanonical: yep\n---\nBody",
        "---\nkind: howto",
    ] {
        let document = parse(source).unwrap();
        assert!(
            !document.frontmatter.as_ref().unwrap().errors.is_empty(),
            "{source}"
        );
        assert_source_ranges(&document);
    }
    for kind in ["generated", "unknown"] {
        let document = parse(&format!("---\nkind: {kind}\n---\nBody")).unwrap();
        assert_eq!(
            document.frontmatter.as_ref().unwrap().kind.as_deref(),
            Some(kind)
        );
        assert!(!document.blocks.is_empty());
    }
}

#[test]
fn quoted_multiline_text_does_not_map_to_container_markers() {
    let source = "> 中文 &amp;\r\n> 日本語です。\r\n\r\n- text\r\n  continues here\r\n\r\n` one\r\ntwo ` and `` `quoted` ``";
    let document = parse(source).unwrap();
    assert_source_ranges(&document);
    let text = fragments(&document)
        .find(|fragment| fragment.text.contains("日本語"))
        .unwrap();
    let start = text.text.find("日").unwrap();
    let span = text
        .source_span(Span {
            start,
            end: start + 3,
        })
        .unwrap();
    assert_eq!(&source[span.start..span.end], "日");
    let code = fragments(&document)
        .find(|fragment| fragment.text == "one two")
        .unwrap_or_else(|| panic!("{document:#?}"));
    let start = code.text.find("two").unwrap();
    let span = code
        .source_span(Span {
            start,
            end: start + 3,
        })
        .unwrap();
    assert_eq!(&source[span.start..span.end], "two");
}

#[test]
fn sentences_keep_versions_urls_and_inline_code_intact() {
    let source =
        "Currently `v1.2.3` works. See [manual.](docs/a.b.md) 次です。你好！ `a! b? c.` stays.";
    let document = parse(source).unwrap();
    assert_eq!(document.sentences.len(), 5);
    assert_eq!(document.sentences[0].fragments[1].text, "v1.2.3");
    assert!(
        document.sentences[1]
            .fragments
            .iter()
            .any(|fragment| fragment.kind == FragmentKind::LinkDestination)
    );
    assert_eq!(document.sentences[4].fragments[0].text, "a! b? c.");
    assert_source_ranges(&document);
}

#[test]
fn mapping_ambiguity_is_explicit_and_not_a_fabricated_offset() {
    let document = parse("![image text](image.png) <email@example.com>").unwrap();
    assert!(
        fragments(&document).any(|fragment| fragment.mapping.iter().any(|segment| !segment.exact))
    );
    assert_source_ranges(&document);
}

#[test]
fn document_cache_roundtrip_is_lossless_and_deterministic() {
    let source = "---\nkind: reference\n---\n# 说明\n\nUse `key` &amp; [manual](a.md).";
    let first = parse(source).unwrap();
    let bytes = serde_json::to_vec(&first).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&parse(source).unwrap()).unwrap());
    assert_eq!(first, serde_json::from_slice::<Document>(&bytes).unwrap());
}

#[test]
fn multilingual_corpus_and_generated_utf8_never_panic_or_break_ranges() {
    for source in [
        include_str!("../docs/design/history.md"),
        "",
        "\0",
        "&NotEqualTilde; &#99999999; &#xFFFFFFFF;",
        "---\r\nkind: [\r\n---\r\n正文。",
    ] {
        assert_source_ranges(&parse(source).unwrap());
    }
    let alphabet = [
        'a', 'Z', '中', '日', 'の', '。', '！', '\r', '\n', '\t', ' ', '[', ']', '(', ')', '<',
        '>', '\\', '&', ';', '#', '-', '`', '*', '\0', '🦀',
    ];
    let mut seed = 0x5eed_u64;
    for length in 0..350 {
        let source: String = (0..length)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                alphabet[((seed >> 32) as usize) % alphabet.len()]
            })
            .collect();
        assert_source_ranges(&parse(&source).unwrap());
    }
}

#[test]
fn document_model_snapshot() {
    let document = parse("---\nkind: howto\nlang: zh\n---\n# 使用\n\n目前 `v1.2.3`。见[文档](ref.md)。\n\n<!-- seiso: allow STL001 -- example -->").unwrap();
    let summary = serde_json::json!({
        "frontmatter": document.frontmatter,
        "sections": document.sections,
        "blocks": document.blocks,
        "links": document.links,
        "comments": document.comments,
        "sentences": document.sentences.iter().map(|sentence| serde_json::json!({
            "span": sentence.span,
            "language": sentence.language,
            "fragments": sentence.fragments.iter().map(|fragment| serde_json::json!({
                "kind": fragment.kind, "text": fragment.text, "span": fragment.span,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    insta::assert_json_snapshot!(summary);
}

#[test]
fn nested_image_and_escaped_definition_labels_map_the_actual_destination() {
    for (source, destination) in [
        ("![a [b](fake.md)](real.md)", "real.md"),
        ("[a\\]:b]: real.md\n\n[a\\]:b]", "real.md"),
        ("[a `]` b](real.md)", "real.md"),
    ] {
        let document = parse(source).unwrap();
        assert_eq!(document.links.len(), 1);
        let link = &document.links[0];
        assert_eq!(link.destination, destination);
        assert_eq!(
            &source[link.destination_span.start..link.destination_span.end],
            destination
        );
        assert_source_ranges(&document);
    }
}

#[test]
fn empty_links_keep_destinations_in_their_sentence() {
    for source in [
        "[](src/config.rs) See source.",
        "[](src/config.rs)",
        "![](diagram.svg)",
        "[ ](src/config.rs)",
    ] {
        let document = parse(source).unwrap();
        assert_eq!(document.links.len(), 1);
        assert_eq!(document.sentences.len(), 1, "{source}");
        assert!(
            fragments(&document).any(|fragment| fragment.kind == FragmentKind::LinkDestination),
            "{source}"
        );
        assert_source_ranges(&document);
    }
}

#[test]
fn thematic_break_and_toml_like_content_are_not_yaml_frontmatter() {
    for source in [
        "---\n# Heading\n",
        "---\nParagraph.\n",
        "+++\nkind = 'howto'\n+++\n\nBody.",
    ] {
        let document = parse(source).unwrap();
        assert!(document.frontmatter.is_none());
        assert_source_ranges(&document);
        if source.starts_with("+++") {
            assert!(fragments(&document).any(|fragment| fragment.text.contains("kind = 'howto'")));
        }
    }
}

#[test]
fn headings_in_quotes_do_not_capture_following_outer_content() {
    let source = "# Outside\n\n> ## Quoted\n> text\n\nMain paragraph.";
    let document = parse(source).unwrap();
    let final_block = document.blocks.last().unwrap();
    assert_eq!(
        document.sections[final_block.section].heading.as_deref(),
        Some("Outside")
    );
    assert!(document.sections[2].span.end < final_block.span.start);
    assert_source_ranges(&document);
}

#[test]
fn markdown_rs_spaced_reference_limitation_retains_original_text() {
    // markdown-rs 1.0 normalizes the first internal space differently when the
    // reference starts with whitespace. Keep the unrecognized syntax as text.
    let source = "[the docs][ A  B ]\n\n[a b]: target.md";
    let tree = markdown::to_mdast(source, &markdown::ParseOptions::default()).unwrap();
    let paragraph = &tree.children().unwrap()[0];
    assert!(matches!(
        &paragraph.children().unwrap()[0],
        markdown::mdast::Node::Text(_)
    ));
    let document = parse(source).unwrap();
    assert!(document.links.is_empty());
    assert_eq!(
        document.sentences[0].fragments[0].text,
        "[the docs][ A  B ]"
    );
    assert_source_ranges(&document);
}

#[test]
fn html_attributes_and_nested_raw_text_do_not_create_suppression_comments() {
    let source = "<div title=\"<!-- seiso: allow VOX001 -- fake -->\">x</div>\n\n<div>\n<script>let x = '<!-- fake -->';</script>\n<style>/* <!-- fake --> */</style>\n<textarea><!-- fake --></textarea>\n<!-- real -->\n</div>\n\nText <script>\"<!-- fake -->\"</script> <!-- inline real --> end.";
    let document = parse(source).unwrap();
    assert_eq!(
        document
            .comments
            .iter()
            .map(|comment| comment.content.as_str())
            .collect::<Vec<_>>(),
        [" real ", " inline real "]
    );
    assert_source_ranges(&document);
}
