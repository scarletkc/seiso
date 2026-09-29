---
kind: reference
---

# Configuration

`seiso init` creates a starting configuration at the repository root;
`seiso init --extend` creates a child overlay in the calling directory.
`seiso policy` shows effective settings and file policy;
[checking documents](../guides/checking.md) covers commands and output. The
accepted fields and defaults are defined by `Settings`,
`LintSettings`, `DupSettings`, `PtrSettings`, and `Lexicon` in
[`src/config/mod.rs`](../../src/config/mod.rs).

## Discovery and inheritance

Each file uses its nearest configuration. In one directory, precedence is
`.seiso.toml`, then `seiso.toml`, then `pyproject.toml` containing `[tool.seiso]`.
Parent configurations are not implicitly merged. `extend = "path/to/base.toml"`
opts into inheritance; the path is relative to the declaring configuration.
Tables merge key by key. Arrays, including `include`, `exclude`, `[[kinds]]`,
and `lint.select`, replace the inherited value instead of adding to it.
`extend-exclude`, `lint.extend-select`, and `lint.extend-ignore` are additive:
their entries from each configuration in the inheritance chain are appended,
in order, to the effective `exclude`, `lint.select`, and `lint.ignore` lists.
`[[extend-kinds]]`, `[[extend-domains]]`, and `[[extend-sites]]` likewise append
to their corresponding mapping lists. Across an inheritance chain, additions
retain parent-to-child order and follow the final ordinary list. Replacing an
ordinary list does not discard inherited additions. For mappings, the last
matching entry wins, so a child's additive entry can override a parent entry.
`seiso policy` reports these effective lists.

Each configuration unit has an **environment frame**: its source file, its
effective policy base, and the relationship by which another unit extends it.
The `extend` filename is always resolved relative to the file declaring it.
On each inheritance edge, an extended configuration that would be selected by
ordinary discovery in a proper ancestor directory is a *governing ancestor*;
its path fields retain that ancestor directory as their base. Any other
extended file is a *shared template*; its path fields use the extending
unit's effective base. The rule is applied independently on every edge, so a
template can extend another configuration without erasing either unit's frame.
For example:

```text
repo/seiso.toml          [[kinds]] path = "docs/**"      (base: repo/)
repo/docs/seiso.toml     extend = "../seiso.toml"       (base: repo/docs/)
repo/docs/guide.md       matches the inherited docs/** entry
```

The frame binds `include`, `exclude`, `extend-exclude`, mapping `path` fields,
`sites.root`, `sites.public`, `[lint.per-file-ignores]` pattern keys, and
`lint.ptr.catalog-dirs`. A `sites.base` value is a URL prefix, not a
filesystem path. With `--config PATH`, one explicitly selected configuration
applies to every file and begins at the shared selection/project root; inheritance edges
still determine the bases of its extended units.

The configuration model distinguishes five roots with different jobs:

| Root | Meaning |
| --- | --- |
| Selection root | The nearest configuration directory, otherwise the nearest `.git` ancestor or calling directory. An unqualified command selects files under this root, and reported filenames are relative to it. |
| Invocation project root | The outermost governing ancestor admitted by the invoking configuration when discovery begins; otherwise the selection root. This is fixed before selected nested configurations are examined. |
| Project/index root | The outermost governing ancestor admitted by the invoking or a selected nested configuration's explicit `extend` chain; otherwise the selection root. It bounds the global dependency index. A shared template does not widen it. |
| Per-source authorized root | The wider of the invocation project root and the root admitted by that file's own governing frames, when one contains the other. Link targets and cross-file comparisons for this source cannot borrow a broader scope admitted *only* by another file. |
| Written `/` link root | For a document under the selected subtree, the selection root; for a dependency outside it, that document's own authorized root. This is independent of a site's `root`/`public` route directories. |

With `--config PATH`, the selection and project roots both use the nearest
`.git` ancestor, otherwise the calling directory. Paths given on the command
line are relative to the caller. An explicit `../sibling/page.md` can be
selected if it stays inside the admitted project; it uses the sibling's own
nearest configuration, not the child's. Paths outside the admitted project
remain invalid. Explicit paths still respect include, exclude, and Git ignore
policy. Unknown fields and unsupported selectors are errors. A standalone
`docs/seiso.toml` can coexist with `docs/sub/seiso.toml` extending the repository
root: preflight of selected files widens the dependency index to that ancestor,
but a neighboring standalone file in `docs/` retains its narrower authorized
root. A wider index is not permission for every indexed file. Conversely, when
invoked from the repository root, a standalone `docs/seiso.toml` does not
remove that invocation's already-admitted access to repository files.

`seiso init --extend` requires a governing ancestor configuration and refuses
to overwrite an existing configuration in the calling directory. It creates
`seiso.toml` with `extend` pointing to that ancestor and puts local suggestions
in additive `extend-exclude`, `[[extend-kinds]]`, and `[[extend-sites]]` entries
instead of replacing the parent's lists. Review the suggestions before
checking the child directory.

In `seiso policy` JSON, `configurations` keeps the effective settings values.
The additional `environments` object has a matching key for each effective
configuration. Its `frames` array gives each frame's `source`, `policy_base`,
`extender` frame index, and `relation` (`selected`, `governing-ancestor`, or
`shared-template`). The `include`, `exclude`, `kinds`, `domains`, `sites`, and
`catalog_dirs` arrays contain frame indices aligned with the corresponding
effective settings entries; `per_file_ignores` maps each pattern key to its
frame index. This exposes interpretation bases without changing settings
strings into objects.

## Kinds and domains

Frontmatter supplies a document kind before path mappings. Kind names are
lowercase and case-sensitive. An invalid
declaration remains invalid instead of inheriting a fallback exemption.
`generated` is available only through configuration. When multiple kind
mappings match, the last wins. Domain mappings use the same precedence;
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
root = "src"           # directory for site route candidates
public = "src/public"  # optional: files served unchanged from /
base = "/"             # optional: URL prefix that links include before the route
```

`path` is a glob; `root` and `public` are directory paths. All three use the
entry's environment-frame base and site directories must stay inside that
source's authorized root, even if an inherited parent site lies outside the selected
child subtree. Such a site can resolve page and public-file targets in parent
or sibling directories within that source's authorized root. An unrelated
nested configuration's broader project scope cannot grant those targets to a
standalone neighbor. A symlink that escapes the source's authorized root does
not make an external file a valid target. Site routing is separate from the
written meaning of a leading `/`: a child document's `/local.md` first names
`child/local.md`, whereas its site route may name a file under the parent's
`sites.root`. When several entries match, the last wins.
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
