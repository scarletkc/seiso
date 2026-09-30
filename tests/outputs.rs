use seiso::diagnostics::{
    Applicability, Diagnostic, Edit, Fix, RelatedLocation, Span, render_github, render_sarif,
};

fn diagnostic() -> Diagnostic {
    let source = "# 日本語\r\n\r\nαβ text\n";
    let start = source.find("αβ").unwrap();
    let mut diagnostic = Diagnostic::new(
        "docs/日本 語#%.md",
        source,
        "SUP002",
        Span::new(start, start + "αβ".len()),
        "Unused comment.",
        "Remove it.",
    );
    diagnostic.url = Some("https://example.com/SUP002".into());
    diagnostic.related.push(RelatedLocation::new(
        "C:\\work\\related.md",
        "関連",
        Span::new(0, 6),
        "Related declaration.",
    ));
    diagnostic.fix = Some(Fix {
        message: "Remove unused text.".into(),
        applicability: Applicability::Safe,
        edits: vec![Edit {
            byte_range: diagnostic.byte_range,
            content: String::new(),
        }],
    });
    diagnostic
}

#[test]
fn rule_documentation_links_are_versioned_and_point_to_existing_pages() {
    let diagnostics: Vec<_> = seiso::rules::rule_codes()
        .map(|code| Diagnostic::new("page.md", "", code, Span::new(0, 0), "Problem", "Fix"))
        .collect();
    let json: serde_json::Value =
        serde_json::from_str(&seiso::diagnostics::render_json(&diagnostics).unwrap()).unwrap();
    let sarif: serde_json::Value =
        serde_json::from_str(&render_sarif(&diagnostics).unwrap()).unwrap();
    let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .unwrap();
    assert_eq!(rules.len(), diagnostics.len());
    for diagnostic in json.as_array().unwrap() {
        let code = diagnostic["code"].as_str().unwrap();
        let path = format!("docs/rules/{code}.md");
        let expected = format!(
            "https://github.com/scarletkc/seiso/blob/v{}/{path}",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(diagnostic["url"], expected);
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(path)
                .is_file()
        );
        let rule = rules.iter().find(|rule| rule["id"] == code).unwrap();
        assert_eq!(rule["helpUri"], expected);
    }
}

#[test]
fn specification_version_is_the_newest_released_version() {
    let changelog = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/CHANGELOG.md"),
    )
    .unwrap();
    let newest = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .find(|heading| *heading != "Unreleased")
        .and_then(|heading| heading.split(" - ").next())
        .unwrap();
    assert_eq!(seiso::SPECIFICATION_VERSION, newest);
    let sarif: serde_json::Value = serde_json::from_str(&render_sarif(&[]).unwrap()).unwrap();
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["properties"]["conventionVersion"],
        newest
    );
}

#[test]
fn unknown_diagnostic_codes_do_not_link_to_nonexistent_rule_pages() {
    let diagnostic = Diagnostic::new("page.md", "", "X001", Span::new(0, 0), "Problem", "Fix");
    assert!(diagnostic.url.is_none());
    let sarif: serde_json::Value =
        serde_json::from_str(&render_sarif(&[diagnostic]).unwrap()).unwrap();
    assert!(
        sarif["runs"][0]["tool"]["driver"]["rules"][0]
            .get("helpUri")
            .is_none()
    );
}

#[test]
fn sarif_preserves_unicode_locations_byte_edits_help_and_related_locations() {
    let diagnostic = diagnostic();
    let value: serde_json::Value =
        serde_json::from_str(&render_sarif(std::slice::from_ref(&diagnostic)).unwrap()).unwrap();
    assert_eq!(value["version"], "2.1.0");
    let run = &value["runs"][0];
    assert_eq!(run["columnKind"], "unicodeCodePoints");
    assert_eq!(
        run["tool"]["driver"]["rules"][0]["helpUri"],
        diagnostic.url.unwrap()
    );
    let result = &run["results"][0];
    let physical = &result["locations"][0]["physicalLocation"];
    assert_eq!(
        physical["artifactLocation"]["uri"],
        "docs/%E6%97%A5%E6%9C%AC%20%E8%AA%9E%23%25.md"
    );
    assert_eq!(physical["region"]["startLine"], 3);
    assert_eq!(physical["region"]["startColumn"], 1);
    assert_eq!(physical["region"]["endColumn"], 3);
    assert_eq!(physical["region"]["byteLength"], 4);
    assert_eq!(
        result["relatedLocations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "file:///C:/work/related.md"
    );
    assert!(
        result["message"]["text"]
            .as_str()
            .unwrap()
            .contains("Suggestion: Remove it.")
    );
    let replacement = &result["fixes"][0]["artifactChanges"][0]["replacements"][0];
    assert_eq!(
        replacement["deletedRegion"]["byteOffset"],
        diagnostic.byte_range.start
    );
    assert_eq!(replacement["deletedRegion"]["byteLength"], 4);
    assert_eq!(replacement["insertedContent"]["binary"], "");
}

#[test]
fn sarif_byte_replacements_encode_exact_utf8_bytes_as_binary() {
    for (content, encoded) in [
        ("", ""),
        ("a", "YQ=="),
        ("ab", "YWI="),
        ("abc", "YWJj"),
        ("αβ", "zrHOsg=="),
        ("汉字", "5rGJ5a2X"),
    ] {
        let mut diagnostic = diagnostic();
        diagnostic.fix.as_mut().unwrap().edits[0].content = content.into();
        let value: serde_json::Value =
            serde_json::from_str(&render_sarif(&[diagnostic]).unwrap()).unwrap();
        let inserted = &value["runs"][0]["results"][0]["fixes"][0]["artifactChanges"][0]["replacements"]
            [0]["insertedContent"];
        assert_eq!(inserted["binary"], encoded);
        assert!(inserted.get("text").is_none());
    }
}

#[test]
fn sarif_paths_are_uri_references_without_accidental_schemes_or_fragments() {
    for (filename, uri) in [
        ("/tmp/文 件.md", "file:///tmp/%E6%96%87%20%E4%BB%B6.md"),
        ("\\\\server\\docs\\page.md", "file://server/docs/page.md"),
        ("a:b?.md", "a%3Ab%3F.md"),
        ("docs/a\nb.md", "docs/a%0Ab.md"),
    ] {
        let mut item = diagnostic();
        item.filename = filename.into();
        let value: serde_json::Value =
            serde_json::from_str(&render_sarif(&[item]).unwrap()).unwrap();
        assert_eq!(
            value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
                ["uri"],
            uri
        );
    }
}

#[test]
fn outputs_are_deterministic_and_sarif_omits_unsafe_fixes() {
    let first = diagnostic();
    let mut second = first.clone();
    second.filename = "a.md".into();
    second.code = "LNK001".into();
    second.fix.as_mut().unwrap().applicability = Applicability::Unsafe;
    let forward = [first.clone(), second.clone()];
    let reverse = [second, first];
    assert_eq!(
        render_sarif(&forward).unwrap(),
        render_sarif(&reverse).unwrap()
    );
    assert_eq!(render_github(&forward), render_github(&reverse));
    let value: serde_json::Value = serde_json::from_str(&render_sarif(&forward).unwrap()).unwrap();
    assert!(value["runs"][0]["results"][0].get("fixes").is_none());
    let empty: serde_json::Value = serde_json::from_str(&render_sarif(&[]).unwrap()).unwrap();
    assert_eq!(empty["runs"][0]["results"], serde_json::json!([]));
    assert_eq!(render_github(&[]), "");
}

#[test]
fn github_escapes_untrusted_properties_and_messages_without_injecting_commands() {
    let mut item = diagnostic();
    item.filename = "C:\\a,b%\r\n::notice::injected.md".into();
    item.code = "SUP002,evil:%\n::notice::".into();
    item.message = "Problem %0A\r\n::error::injected".into();
    item.suggestion = "Fix\n::warning::".into();
    let rendered = render_github(&[item]);
    assert_eq!(rendered.lines().count(), 1);
    assert!(
        rendered.starts_with("::error file=C%3A\\a%2Cb%25%0D%0A%3A%3Anotice%3A%3Ainjected.md,")
    );
    assert!(rendered.contains("title=SUP002%2Cevil%3A%25%0A%3A%3Anotice%3A%3A"));
    assert!(rendered.contains("Problem %250A%0D%0A::error::injected"));
    assert!(rendered.contains(",col=1,endColumn=2::"));
    assert!(rendered.contains("%0ARelated: C:\\work\\related.md:1:1:"));
}

#[test]
fn github_uses_line_ranges_for_multiline_diagnostics_and_keeps_zero_width_valid() {
    let multiline = Diagnostic::new(
        "page.md",
        "one\ntwo",
        "X001",
        Span::new(0, 7),
        "Problem",
        "Fix",
    );
    let rendered = render_github(&[multiline]);
    assert!(rendered.contains(",line=1,endLine=2,title=X001::"));
    assert!(!rendered.contains(",col="));
    let point = Diagnostic::new("page.md", "", "X001", Span::new(0, 0), "Problem", "Fix");
    assert!(render_github(&[point]).contains(",col=1,endColumn=1::"));
}
