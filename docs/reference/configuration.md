---
kind: reference
---

# Configuration

`seiso init` creates a starting configuration at the repository root. `seiso policy` shows effective
settings and file policy; [checking documents](../guides/checking.md) covers commands and
output. The accepted fields and defaults are defined by `Settings`,
`LintSettings`, `DupSettings`, `PtrSettings`, and `Lexicon` in
[`src/config/mod.rs`](../../src/config/mod.rs).

## Discovery and workspace roots

Each file uses its nearest configuration. In one directory, precedence is
`.seiso.toml`, then `seiso.toml`, then `pyproject.toml` containing `[tool.seiso]`.
Parent configurations are not implicitly merged. `extend = "path/to/base.toml"`
opts into inheritance; the path is relative to the declaring configuration.

The workspace root is the nearest configuration's directory. When that
configuration explicitly inherits a governing ancestor configuration, the
workspace root is that ancestor's directory, following the inheritance chain.
This preserves discovery, Git ignore coverage, and site pages outside the child
directory. Each discovered file still uses its own nearest configuration.
Extending a reusable template does not expand the workspace.
Without a configuration, the root is the nearest ancestor containing `.git`,
otherwise the calling directory. With `--config`, the root is the nearest
ancestor containing `.git`, otherwise the calling directory.

Paths given on the command line are relative to the caller; reported paths
are relative to the workspace. With no explicit paths, checks cover the whole
workspace. Explicit paths still respect include, exclude, and Git ignore policy.
Unknown fields and unsupported selectors are errors.

## Merging

Tables merge key by key. Arrays, including `include`, `exclude`, `[[kinds]]`,
and `lint.select`, replace the inherited value instead of adding to it.
`extend-exclude`, `lint.extend-select`, and `lint.extend-ignore` are additive:
their entries from each configuration in the inheritance chain are appended,
in order, to the effective `exclude`, `lint.select`, and `lint.ignore` lists.
If a child replaces one of those base arrays, the inherited additions still
apply. `seiso policy` reports the effective lists.

## Path bases

Local path entries are relative to the selected configuration's directory.
When `extend` names the governing configuration in an ancestor directory
(the file discovery would select there), its inherited path entries retain
that ancestor's base. For example, `docs/seiso.toml` extending `../seiso.toml`
keeps the parent's `exclude = ["docs/generated/**"]` effective under `docs/`.
This applies to include/exclude patterns, kind/domain/site mappings,
per-file ignores, and catalog directories. Site roots and public directories
use the same base as their mapping's path.

Other extended files are reusable templates: their entries resolve from the
extending configuration's effective base, even if the template lives in a
subdirectory. A template inherited by a governing parent first takes that
parent's base, which is then retained by children. Local replacements use
the child's base; per-file-ignore keys merge and keep their individual bases.

`--config PATH` selects one configuration for every file. Its own entries
are relative to the workspace root; entries inherited from a governing
ancestor still retain that ancestor's base. `seiso policy` reports
`pattern_bases` for every effective configuration: each entry includes its
original `pattern` and `base_directory`, relative to the inspected workspace
(`.` for the workspace root, `..` for its parent).

## Kinds and domains

Kind declaration and assignment are defined by `KIND-2` to `KIND-4` in the
[specification](../../spec/convention.md#document-kinds); seiso expresses
path mappings as `[[kinds]]` entries. Kind names are lowercase and
case-sensitive. When multiple `[[kinds]]` entries match, the last wins.
`[[domains]]` also uses the last matching entry; the workspace is the default
comparison domain.

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

[[kinds]]
path = "**/AGENTS.md"
kind = "agents"

[[domains]]
path = "docs/client/**"
name = "client"

[[domains]]
path = "docs/server/**"
name = "server"
```

## Documentation sites

Site generators such as VitePress, Docusaurus, mdBook, and MkDocs publish
pages at routes, so their sources often link to `/guide/setup` for
`src/guide/setup.md`, or to `setup.html` for `setup.md`. A `[[sites]]` entry
lets LNK001 and LNK002 resolve those links as routes for the documents it
matches. Without one, links resolve only as repository paths, the way GitHub
renders them.

```toml
[[sites]]
path = "src/**"        # documents the site renders
root = "src"           # directory that a leading / resolves from
public = "src/public"  # optional: files served unchanged from /
base = "/"             # optional: URL prefix that links include before the route
```

`path` is a glob; `root` and `public` are directories inside the workspace.
All three use the mapping's [path base](#path-bases). When several entries
match, the last wins.
Set `base` only when links include a prefix before the route, such as
`/docs/` in `/docs/guide/setup`. [LNK001](../rules/LNK001.md#inputs) lists
the route candidates in the order they are tried.

`seiso init` suggests an entry for each VitePress, Docusaurus, mdBook, or
MkDocs project it finds at the repository root or one directory below it.
`seiso policy` and `seiso index --dump` show the site that applies to each file.

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
full code such as `KND001`. `select` defaults to `ALL`; `--select` replaces it
and any `lint.extend-select` entries, and `--extend-select` adds selectors.
Selection and ignore conflicts use the
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
