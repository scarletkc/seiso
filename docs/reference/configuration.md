---
kind: reference
---

# Configuration

`seiso init` creates a starting configuration at the repository root. `seiso policy` shows effective
settings and file policy; [checking documents](../guides/checking.md) covers commands and
output. The accepted fields and defaults are defined by `Settings`,
`LintSettings`, `DupSettings`, `PtrSettings`, and `Lexicon` in
[`src/config/mod.rs`](../../src/config/mod.rs).

## Discovery and inheritance

Each file uses its nearest configuration. In one directory, precedence is
`.seiso.toml`, then `seiso.toml`, then `pyproject.toml` containing `[tool.seiso]`.
Parent configurations are not implicitly merged. `extend = "path/to/base.toml"`
opts into inheritance; the path is relative to the declaring configuration.
Tables merge key by key. Arrays, including `include`, `exclude`, `[[kinds]]`,
and `lint.select`, replace the inherited value instead of adding to it.
Effective glob patterns, including inherited patterns, are relative to the
selected configuration's directory. `--config PATH` selects one configuration
for every file; its glob patterns are relative to the workspace root.

The workspace root is the directory of the nearest configuration, otherwise
the nearest ancestor containing `.git`, otherwise the calling directory. With
`--config`, the root is the nearest ancestor containing `.git`, otherwise the
calling directory. Paths
given on the command line are relative to the caller; reported paths are
relative to the workspace. Explicit paths still respect include, exclude, and
Git ignore policy. Unknown fields and unsupported selectors are errors.

## Kinds and domains

Frontmatter supplies a document kind before path mappings. Kind names are
lowercase and case-sensitive. An invalid
declaration remains invalid instead of inheriting a fallback exemption.
`generated` is available only through configuration. When multiple `[[kinds]]`
entries match, the last wins. `[[domains]]` also uses the last matching entry;
the workspace is the default comparison domain.

```toml
include = ["**/*.md"]
exclude = ["vendor/**"]

[[kinds]]
path = "docs/**"
kind = "howto"

[[kinds]]
path = "docs/reference/**"
kind = "reference"

[[kinds]]
path = "docs/api/generated/**"
kind = "generated"

[[kinds]]
path = "README.md"
kind = "readme"

[[domains]]
path = "docs/client/**"
name = "client"

[[domains]]
path = "docs/server/**"
name = "server"
```

## Document language

Language-specific rules match each sentence with the English, Chinese, or
Japanese lexicon. A sentence is Chinese or Japanese when its CJK characters
outnumber its Latin words, and Japanese when it contains kana. Inline code
and link destinations do not count. The document's language is detected from
all of its prose the same way; structure rules use it, and duplicate checks
compare only documents in the same language.

`lang: en`, `lang: zh`, or `lang: ja` in frontmatter sets the language of the
document and every sentence in it. `lint.languages` selects which languages'
rules run; it does not change detection.

## Rule selection

Selectors accept `ALL`, an implemented family such as `KND`, or an implemented
full code such as `KND001`. `select` defaults to `ALL`; `--select` replaces it,
and `--extend-select` adds selectors. Selection and ignore conflicts use the
more specific entry; equal specificity favors ignore. Preview rules are then
removed unless `preview = true` or `--preview` is set. Per-file ignores and
kind applicability further restrict enabled rules. Suppression is applied
after diagnostics are produced.

```toml
preview = false

[lint]
select = ["ALL"]
ignore = ["PTR003"]
languages = ["en", "zh", "ja"]

[lint.per-file-ignores]
"docs/legacy/**" = ["DUP"]

[lint.ptr]
catalog-dirs = ["locales/", "migrations/"]

[lint.lexicon.zh]
extend-stale-markers = ["截至目前"]
```

Rule documentation identifies each rule's thresholds and word-list options.
The listed lexicon fields can be extended independently for English, Chinese, and Japanese;
Chinese and Japanese matching does not require word segmentation. Thresholds
are calibrated on tuning data under the [evaluation policy](../evaluation/policy.md).
