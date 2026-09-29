# Unreleased: configuration environment frames

This change addresses the path-base defect in [#40](https://github.com/scarletkc/seiso/issues/40)
and adds the additive child-overlay workflow in
[#39](https://github.com/scarletkc/seiso/issues/39). It is not part of a
numbered release yet.

## New

- `[[extend-kinds]]`, `[[extend-domains]]`, and `[[extend-sites]]` append mapping
  entries across an explicit inheritance chain. Ordinary `[[kinds]]`,
  `[[domains]]`, and `[[sites]]` arrays still replace their inherited ordinary
  arrays. Additions follow the effective ordinary list in parent-to-child
  order, and the last matching mapping wins.
- `seiso init --extend`, run in a child directory, creates a `seiso.toml` that
  extends its nearest governing ancestor configuration. It uses additive
  entries for local suggestions and refuses to overwrite an existing config.
- `seiso policy` JSON gains `environments`, keyed like `configurations`. Each
  environment lists source/base/relationship frames and frame indices aligned
  with effective path-bearing settings. Existing `configurations` arrays and
  string values retain their shape.

## Behavior change and upgrade guidance

Inherited paths now use the environment frame of the configuration unit that
declared them. A governing ancestor's patterns and site directories retain
that ancestor's directory as their base; a shared template still uses the
extending configuration's effective base. This corrects parent policies that
previously stopped matching under child configs, but it **can change which
files are included, excluded, classified, ignored, or assigned to a site**.
Review `seiso policy` before and after upgrading, especially where a child
configuration extends a governing ancestor and previously relied on rebased
parent patterns. Relevant fields are `include`, `exclude`,
`extend-exclude`, kind/domain/site `path`, `sites.root`, `sites.public`,
per-file ignore patterns, and `lint.ptr.catalog-dirs`.

An explicit governing-ancestor `extend` chain can admit a project/index root
wider than the child invocation's selection root. Preflight also examines
selected files' nested configurations: even if `docs/seiso.toml` is standalone,
`docs/sub/seiso.toml` can explicitly inherit a repository-root configuration
and widen the dependency index. An unqualified child command still selects
and reports files relative to `docs/`. The index's wider scope is **not a
permission shared by all sources**: links and cross-file comparisons use the
wider of the original invocation project's root and each source's own
governing-frame root, when those roots are nested. A standalone neighbor in
`docs/` cannot follow a parent link or compare against parent files merely
because the nested file imported them. Conversely, when invoked from the
repository root, a standalone `docs/` config retains the root scope already
admitted by that invocation.

Within a source's admitted scope, site routes and cross-file dependencies can
resolve parent and sibling targets; symlinks outside that scope remain
excluded. An explicit `../sibling/page.md` argument may select a file within
the admitted project, using that file's own nearest configuration. When a
selected policy enables index-dependent rules, a child check may load a wider
dependency pool without reporting unrelated parent or sibling diagnostics.
Written leading `/` links in selected child documents continue to use the
child selection root; a site's `root`/`public` routing bases are independent.

For the model and examples, see the [configuration reference](../reference/configuration.md).

The current project/index model widens along governing **ancestor** directories;
it does not create a second, disjoint filesystem mount merely because an
external shared-template file was imported. A site directory outside the
source's admitted root remains a configuration error rather than silently
becoming a readable target. This is a deliberate single-root boundary, not a
promise that every external template directory will be scanned.
