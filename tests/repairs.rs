//! Authored repair regressions; these are not an independent agent or human repair study.
mod common;
use common::{CheckContext, check};

use seiso::config::{CliOverrides, Config};

#[test]
fn suggested_repairs_clear_each_rule_and_preserve_unaffected_facts() {
    let root = std::env::current_dir().unwrap();
    let protected = "\n\n## Connection contract\n\nSet `SERVICE_PORT` to `8080`. Read the [API contract](https://example.org/api#connection).\n";
    for code in [
        "STL002", "STL004", "RAT001", "ORD001", "ORD002", "MIX001", "VOX002", "VOX003", "EVD001",
    ] {
        let documentation = seiso::rules::rule(code)
            .unwrap()
            .documentation
            .replace("\r\n", "\n");
        let examples: Vec<_> = documentation
            .split("```markdown\n")
            .skip(1)
            .map(|text| text.split("```").next().unwrap())
            .collect();
        let config = Config::parse(
            "preview=true\n[lint]\nselect=['STL002','STL004','RAT001','ORD001','ORD002','MIX001','VOX002','VOX003','EVD001']",
            &root,
        ).unwrap();
        let documents: Vec<_> = examples
            .iter()
            .map(|source| seiso::md::parse(&(source.to_string() + protected)).unwrap())
            .collect();
        for (index, document) in documents.iter().enumerate() {
            let result = check(&CheckContext {
                document,
                filename: "guide.md",
                path: &root.join("guide.md"),
                workspace_root: &root,
                config: &config,
                overrides: &CliOverrides::default(),
            })
            .unwrap();
            assert_eq!(
                result.diagnostics.is_empty(),
                index == 1,
                "{code}: {:?}",
                result.diagnostics
            );
            assert!(document.source.ends_with(protected));
        }
        let facts = |document: &seiso::md::Document| {
            let start = document.source.len() - protected.len();
            let ids: Vec<_> = document
                .identifiers
                .iter()
                .filter(|id| id.span.start >= start)
                .map(|id| id.text.clone())
                .collect();
            let links: Vec<_> = document
                .links
                .iter()
                .filter(|link| link.span.start >= start)
                .map(|link| link.destination.clone())
                .collect();
            (ids, links)
        };
        assert_eq!(facts(&documents[0]), facts(&documents[1]), "{code}");
        assert!(!facts(&documents[0]).0.is_empty());
        assert!(!facts(&documents[0]).1.is_empty());
    }
}
