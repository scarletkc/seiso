# Per-configuration environment frames for inheritance (#40, then #39)

Status: design decision and implementation guide, 2026-09-29. Code baseline:
`origin/main` `3fb1baa`. This document refines
[the issue review](config-inheritance-issues-39-40.md)
against the current implementation and the actual [#40] and [#39] contracts.
It records no measured performance improvement and is not an implementation.

## The invariant and the current failure

For a file below a child configuration, a path-bearing declaration from a
governing ancestor must denote the same physical path as before the child was
installed. A declaration from a shared template remains reusable: its path
denotes a location relative to the configuration context that imports it. This
is a property of each `extend` **edge**, not of a string's syntax or the file's
extension. `--config` remains a separate entry mode, whose selected file starts
at the workspace root. The public TOML and current `Settings`/policy JSON field
shapes should remain stable.

Today `load_extended` merges unannotated `toml::Value`s, then `from_value`
deserializes and compiles every surviving pattern. `relative_path` supplies only
the selected configuration directory to all matchers (`src/config/mod.rs`),
so the origin context has already been erased. The fix should establish the
interpretation environment before lowering any path-bearing declaration.

## Minimal semantic model

Let `S(C)` be the normalized absolute path of configuration unit `C`, `D(C)`
its lexical parent directory, `B(C)` its effective policy base, and `W` the
invocation's workspace root. `S` answers *where was `extend` written?*;
`B` answers *where do this unit's policy-relative fields point?*; `W` answers
*which files may this invocation inspect?*. These three roles must not be
collapsed. A frame is immutable and may share `W` with its neighbors:

```rust
/// Context in which one configuration unit is elaborated.
struct Frame {
    source: PathBuf,
    policy_base: PathBuf,
    workspace_root: PathBuf,
    incoming: ExtendRelation,
}

/// Why the extending unit chose the extended unit's interpretation base.
enum ExtendRelation { Selected, GoverningAncestor, Template }

/// A surviving declaration and the environment that gives it meaning.
struct Located<T> { value: T, frame: FrameId }
```

`FrameId` can index an interned per-load frame table; an `Arc<Frame>` also
works, but IDs make policy provenance and grouping compact. The relation is
metadata about the incoming edge, not a runtime branch at every matcher.

Given extending unit `C` with frame `F_C` and its `extend` target `P`, define
`governs(P,C)` iff **(1)** `D(P)` is a proper lexical ancestor of `D(C)`, and
**(2)** ordinary configuration discovery in `D(P)` would select exactly `P`
under the existing `.seiso.toml > seiso.toml > [tool.seiso]` priority. Then:

```text
target path P          = normalize(D(C) / literal extend path)
source(P)              = P                  (always: even templates)
base(P)                = D(P)               if governs(P,C)
                       = base(C)            otherwise
workspace(P)           = workspace(C)       (no implicit root promotion)
```

The exact-path comparison in (2) should use normalized lexical absolute paths,
not merely canonical file identity. A symlink alias to a parent configuration
is an explicit alternate reference and should not silently acquire governing
semantics; canonical identity remains useful for cycle detection. This choice
needs a Windows case-folding test, since lexical strings need not reflect
filesystem case identity. A same-directory lower-priority configuration and a
template in a descendant directory are both `Template` edges and thus keep the
old selected-base behavior. A grandparent's selected configuration qualifies
even if the child skips an intermediate configuration: #40 says a configuration
governing **an ancestor directory**, not necessarily the nearest parent.

The selected frame begins with `B = D(C)` for normal discovery, or `B = W`
for `--config`. `Config::parse`/defaults use their caller-supplied directory.
The relation is recomputed for *every* edge, not inherited as a flag. Example:

```text
repo/shared/base.toml --(Template, B=repo)--> repo/seiso.toml
repo/seiso.toml      --(GoverningAncestor, B=repo)--> repo/docs/seiso.toml
repo/docs/seiso.toml --(Selected, B=repo/docs)
```

The arrows above show the values flowing toward the selected child, not the
direction of `extend` strings. A template can itself extend another template
or its own governing ancestor; each hop makes its own decision from its
physical source path and its caller's effective base.

## Compiler pipeline and representation

1. **Discover the selected unit and invocation workspace.** Retain today's
   discovery and `--config` distinction. Build its frame before inspecting
   semantic fields.
2. **Parse one source unit as syntax.** `read_document` still extracts
   `[tool.seiso]`; parse TOML, remove and validate the *local* `extend`, and
   resolve its target against `source.parent()`, never against `policy_base`.
   Classify the edge and recursively parse its target with its new frame.
3. **Overlay source-tagged syntax.** Each surviving leaf (or each atomic
   array element/table entry) retains the frame that supplied it. Ordinary
   tables merge by key; ordinary arrays replace; `extend-exclude`,
   `lint.extend-select`, and `lint.extend-ignore` accumulate across the chain.
   For #39, `extend-kinds`, `extend-domains`, `extend-sites` accumulate the
   same way. Maintain separate effective *base list* and *extension list*;
   lowering appends extension entries after the final base list. A child's
   plain base-list replacement therefore does not erase inherited extension
   entries, matching the existing `extend-*` precedent and #39's decision.
4. **Elaborate surviving syntax to typed policy.** Deserialize the effective
   user-facing `Settings` and validate as today, but compile path-bearing
   entries with their own frames. A `BoundGlob` carries matcher + base +
   source; a `BoundSite` carries the path matcher and resolved `root/public`
   from the same source entry, while `base` remains a URL path. Preserve the
   raw strings in `Settings` for established reporting/serialization.
5. **Evaluate files.** Match a normalized absolute file only after deriving
   its path relative to each bound base. `include`/`exclude` use any-match;
   kinds/domains/sites scan in reverse order for last-match-wins. `site_routes`
   consumes the resolved directories, not `self.directory.join(raw_string)`.

The source-tagged syntax stage is important for compatibility. The old
`toml::Value` overlay can discard a malformed *overridden* ancestor field
before deserialization and validation. Deserializing and validating every unit
eagerly would make previously accepted configurations fail. Retain syntax
errors and `extend` errors per unit, but defer final field validation until
after overlay, unless a deliberate separate breaking change is approved. This
is still compiler-style elaboration: the frame is chosen before meaning is
assigned, even though syntax parsing precedes semantic interpretation.

Path-bearing fields in the current schema are `include`, `exclude`,
`extend-exclude`, `kinds.path`, `domains.path`, `sites.path`, `sites.root`,
`sites.public`, `lint.per-file-ignores` keys, and
`lint.ptr.catalog-dirs`. The latter is currently resolved by
`file.config.directory.join(catalog)` in `src/rules/cross_file.rs:138`, so
it is affected by the same lost-frame defect even though #40 does not list
it explicitly; include it in the fix or record a deliberate scope exception.
`sites.base` is a URL prefix, not a
filesystem path; selectors, languages, and lexicon strings are not paths.
Do not infer path semantics from the fact that a field happens to be `String`.
Keep the path-field classification in one typed lowering module, with tests
that enumerate all path-bearing schema fields, rather than scattering
`if field == ...` checks through matching and merge code.

For `lint.per-file-ignores`, a child redeclaring the **same string key**
replaces its selectors and its frame, exactly as the current table overlay
does. Different keys retain their independent frames. Diagnostic labels
should use each surviving value's source; `policy` should add a separate
deterministic provenance record aligned with effective entries, rather than
changing JSON string arrays into objects.

## Workspace boundary: a real non-local conflict

The frame model preserves a parent's site `root/public` referents, but this
does not by itself preserve all site behavior when a user changes invocation
directory. `Workspace::discover` makes the nearest config directory the
workspace root. From `repo/docs`, a parent-bound `root = "."` denotes `repo`,
outside the invocation root `repo/docs`; `with_sites_inside` currently rejects
it. More importantly, `workspace::load` walks and names files relative to its
workspace root, and `local_link_targets` deliberately omits route candidates
outside it. Merely rebasing patterns cannot make out-of-workspace files
available to checks.

**Superseded after the user's 2026-09-29 clarification:** keeping `W` at the
child and allowing a parent site directory only as a routing base would
silently lose sibling/public targets. Explicitly extending a governing parent
means those targets are part of the project. The project-resolution boundary
must therefore include them. See the dated addendum below for the replacement
design, which separates project, scan/selection, and presentation scopes.

Simply retaining `with_sites_inside` unchanged is invalid: an inherited parent
site would fail before policy inspection from the child directory. Promoting
the single old `Workspace.root` field without separating its roles is also
invalid: it would misapply the child config at the root and widen default
selection/output unintentionally.

## Long-tail performance without a second semantic model

The dominant path should remain cheap: no `extend` and few patterns yield one
frame, one compiled policy, and the same simple matching loop. The expensive
tails are long chains (already capped at 128), many files sharing a selected
config, and many patterns from distinct frames. Optimizations should preserve
the exact per-entry semantics:

* Within one snapshot, memoize `config_in(directory)` and compiled effective
  configs by selected path **plus policy base and workspace root**. A source
  path alone is not a valid key: the same template can be imported in several
  contexts. Avoid re-reading/recompiling a chain per file in `config_for`.
  Snapshot-scoped caches need no filesystem invalidation protocol; a future
  daemon must version file contents/metadata and discovered-directory state.
* Intern frames and bases; for each candidate file, compute each distinct
  base-relative string once, then evaluate its bound patterns. This changes
  repeated path stripping from roughly `O(patterns × path depth)` to
  `O(distinct bases × path depth + patterns)`. Avoid rewriting glob text,
  which is brittle around metacharacters and separators.
* Compile once per effective config and preserve reverse-order early exit for
  last-match mappings. Do not build a global automaton until a benchmark shows
  matcher count is the bottleneck: precedence and heterogeneous bases make a
  blind aggregate set easy to get wrong.
* Bound chain depth and cycle detection before recursion; memoize discovery
  lookups along a chain, but do not cache partially elaborated results keyed
  only by file path. Template elaboration is context-sensitive.

Measure cold and warm snapshot time, configuration I/O count, glob compilation
count, peak memory, and per-file policy latency on fixtures with: 1) no
inheritance, 2) a 64/128-unit chain, 3) 10k files under one child config,
4) one template imported from many child bases, and 5) thousands of mixed-base
mapping entries. Compare policies byte-for-byte as well as timing. No
specific speedup is asserted without these measurements.

## Implementation order and verification

1. Introduce frame construction and tagged overlay with no change to public
   TOML syntax; differential-test all existing fixtures for the non-governing
   template cases and `--config`.
2. Bind every existing path-bearing policy entry, switch matchers/site routes
   to use bound values, and add policy provenance without removing or changing
   `Settings` JSON fields. Test parent/child and three-level mixed-edge chains.
3. Resolve the inherited-site workspace conflict explicitly, with tests both
   from the repository root and child directory. Never silently read files
   outside `W` merely because a parent-bound route points there.
4. Add #39's three `extend-*` mapping arrays on the same tagged overlay;
   verify base replacement plus inherited extension ordering and last-match
   precedence. Then implement `init --extend` and compare per-file policy
   before/after under a fixed invocation root.
5. Add the snapshot-local caches and long-tail benchmarks, retaining a
   no-cache reference path in tests for differential correctness.

The central falsifiable invariant is:

```text
For any file f under child C and fixed W, every surviving declaration d
from a governing ancestor P evaluates at f under B(P)=D(P), regardless of C.
Every declaration d from a non-governing template T evaluates at f under
B(T)=B(the unit that extends T), recursively per extend edge.
```

This isolates the path-scoping claim from separate changes in workspace
selection and list precedence.

## External evidence and limits

* [Git's `.gitignore` manual] keeps patterns associated with the directory of
  the file that supplied them. This is production evidence for source-aware
  matching, **not** for seiso's template-rebasing rule.
* [Ruff's configuration documentation] uses closest-config discovery,
  explicit inheritance, and a selected project root for inherited paths. It
  demonstrates the shared-template use case but does not solve #40's
  governing-ancestor distinction.
* [Néron et al., *A Theory of Name Resolution*] models environments and
  relations between scopes rather than reducing every reference to an
  uncontextualized string. The analogy motivates explicit frame/edge models;
  it is not an empirical performance claim about seiso.
* [Mokhov et al., *Build Systems à la Carte*] and related incremental-build
  work motivate dependency-aware reuse; seiso should start with an
  invocation-local cache because its workload does not yet justify a general
  incremental engine.

[#39]: https://github.com/scarletkc/seiso/issues/39
[#40]: https://github.com/scarletkc/seiso/issues/40
[Git's `.gitignore` manual]: https://git-scm.com/docs/gitignore
[Ruff's configuration documentation]: https://docs.astral.sh/ruff/configuration/
[Néron et al., *A Theory of Name Resolution*]: https://doi.org/10.1007/978-3-662-46669-8_9
[Mokhov et al., *Build Systems à la Carte*]: https://www.cambridge.org/core/journals/journal-of-functional-programming/article/build-systems-a-la-carte-theory-and-practice/097CE52C750E69BD16B78C318754C7A4

## Addendum — 2026-09-29: an explicitly inherited parent is project scope

The user clarified that a configuration explicitly extended from outside the
child's old workspace is part of the project; its parent/sibling site targets
must be usable, not merely named in policy output. This supersedes the
fixed-child-`W` recommendation above. The underlying issue is that today's
single `Workspace.root` plays at least four unrelated roles: a resolution and
security boundary, a default scan root, the base of written `/` links, and the
origin of user-visible filenames. Extending the project does **not** require
all four to grow together.

### Proposed state and representative workflow

For normal discovery, retain the nearest configuration directory as
`selection_root`. Load its framed `extend` chain, then choose `project_root`
as the outermost governing-ancestor frame base that contains
`selection_root` (or `selection_root` when none exists). `report_root` starts
as `selection_root`; `--config` retains its current repository-root selection
and pattern base. Conceptually:

```rust
/// Different directory roles for one invocation; none is inferred from another.
struct ProjectContext {
    project_root: PathBuf,   // Internal index, route containment, dependency universe.
    selection_root: PathBuf, // Default files the command checks and reports.
    report_root: PathBuf,    // Presentation base for selected-file names.
}

/// Link syntax and link safety intentionally use different roots.
struct LinkContext<'a> {
    written_root: &'a Path,  // Base for a leading '/' in the Markdown source.
    project: &'a ProjectContext,
    site: Option<&'a SiteRoutes>,
}
```

For `repo/docs/seiso.toml -> repo/seiso.toml`, the three main values are
`project_root=repo`, `selection_root=repo/docs`, and `report_root=repo/docs`.
The parent's site `root=repo`, `public=repo/public` are inside the project.
`seiso check` from `docs` still selects only `docs/**` by default, but can
inspect `repo/public/logo.svg` or a sibling page as a site target and can
index parent/sibling Markdown when an enabled cross-file rule needs it.
An explicit path `../README.md` is now inside this declared project and may
be selected; this is a deliberate additive change, not an accidental escape.

The selected child config and the project-root config are distinct objects.
After promotion, `Workspace::config_for(file)` must discover the closest
config for **that file** within the project and may use an invocation-local
directory cache. Its current `directory == self.root => self.config.clone()`
shortcut would incorrectly apply the child config to a root file. The
selected config can still be retained separately for provenance, but it must
not stand in for project-root policy. A sibling's own configuration remains
authoritative for sibling files; extending one parent does not impose the
child's overrides on siblings.

### Resolution, discovery, and output are separate operations

1. **Resolution/security:** Use `project_root` for internal index IDs,
   workspace-relative target lookup, and canonical containment. A parent or
   sibling target inside the project is no longer skipped by
   `local_link_targets` or rejected by `local_target_status`. Continue
   rejecting symlinks that resolve outside the declared project; extending a
   parent is not permission to read arbitrary paths. Site `root/public` must
   be resolved from their declaration frames before containment checks.
2. **Default discovery:** Initially walk `selection_root`, not
   `project_root`, so a bare `check` does not suddenly check every parent and
   sibling document. If selected policies require cross-file indexing, add a
   project-wide dependency pass and deduplicate already seen files by
   absolute path. `policy` and `index --dump` should present the child scope
   by default even if a wider index is built internally. `--config` retains
   its existing default scan behavior. The cache directory should likewise
   stay under the original invocation/selection root unless changed
   explicitly.
3. **Written root-relative links:** `paths::local_link_targets(root, ...)`
   currently uses `root` both to interpret a written `/foo` and to bound
   target/index paths. Split those parameters. For selected child files,
   keep the old written-link base `selection_root`; for dependency files
   outside the selected subtree, use `project_root`. Site routing uses its
   independently bound absolute `site.root/public`. Without this split,
   promoting the index root would silently reinterpret an existing child
   `/foo` as `repo/foo` instead of `repo/docs/foo`.
4. **Presentation:** Internal `IndexedFile.filename`, `Snapshot.selected`,
   lookup keys, cross-file references, and fix application should use stable
   project-relative IDs. Convert copies to `report_root`-relative names only
   at output boundaries: diagnostic filename and related locations,
   `PolicyReport` file/configuration keys, index dump, errors, and statistics.
   Selected child paths retain their old spelling (`guide.md`, not
   `docs/guide.md`). A newly visible parent/sibling path can be rendered as
   `../README.md` or an explicit project-relative name with a declared base;
   choose one representation consistently and document it. Do not pass a
   display name back into index lookup or filesystem operations.

Cross-file diagnostics may legitimately change: an explicitly inherited
project can reveal parent/sibling comparisons that were previously outside
the child workspace. That is a semantic consequence to test and release-note,
not a reason to reject the inherited site. Conversely, a non-index single-file
check should not eagerly parse the entire project merely to validate a route
that can be checked by filesystem status.

### Long-tail topology and containment

The common case is one ancestor chain, so a single `project_root` is exact and
cheap. A rare chain can import a shared template outside the repository which
itself extends a governing ancestor in *its* tree. Per-edge frame semantics
then produce two disjoint policy bases. Do **not** choose their filesystem
least-common ancestor (possibly a drive root) and recursively scan it; that
would be a severe performance and security regression. Define project scope
generally as a bounded union of roots admitted by effective governing frames.
Canonicalize each admitted root once for containment; eliminate roots wholly
contained in another admitted root. The single-ancestor-root case is a fast
path of this model, not a different semantic rule. A shared template's
physical source directory alone does not admit a data tree; only a
governing-frame base whose declarations actually refer to that tree does.

The general internal file identity should be an absolute normalized path or
a `(mount_id, relative_path)` pair. Discovery walks only admitted mounts and
only when active rules need their files; target lookup can visit a specific
admitted path without walking unrelated directories. Keep the written `/`
base per source file, and preserve user-facing child-relative paths in the
primary mount. Newly visible external-mount files need an unambiguous
presentation (for example, a deterministic mount prefix plus root metadata).
The current single-root `WorkspaceIndex` cannot fully model cross-volume
mounts, so merely relaxing `with_sites_inside` is not completion. Build the
single-root fast path first, then the bounded multi-root path, before claiming
the general explicitly inherited semantics. Do not silently treat disjoint
targets as missing, reject them as a matter of policy, or scan a drive root.

### Acceptance probe

Fixture: `repo/seiso.toml` declares `[[sites]] path="docs/**",
root=".", public="public"`; `repo/docs/seiso.toml` extends it; `docs/a.md`
links to a sibling page and `/logo.svg` in `public`. Run `check` from `docs`
with and without cross-file link rules. The selected check set and displayed
name for `a.md` remain child-relative; both targets resolve; no unrelated
root document is checked by default; `--config` behavior is unchanged; a
symlink from `public` outside the project remains inaccessible. A second
fixture with unrelated external template roots verifies that the loader never
walks their least-common ancestor.

## Addendum — 2026-09-29: nested configs require per-file scope discovery

A further concrete counterexample invalidates deriving project scope only
from the **invocation-selected** configuration:

```text
repo/seiso.toml             # site root='.', public='public'
repo/docs/seiso.toml        # standalone; selected when invoked from docs/
repo/docs/a.md              # uses the standalone docs config
repo/docs/sub/seiso.toml    # extend='../../seiso.toml'
repo/docs/sub/b.md          # uses the parent-bound site/public paths
repo/public/logo.svg
```

`Workspace::discover(docs)` sees the standalone `docs/seiso.toml`. If it fixes
`project_root=docs` before `config_for(b.md)` loads the nested config, `b.md`
cannot use its explicitly inherited parent site. Conversely, changing one
global root to `repo` would incorrectly authorize `a.md` to read parent or
sibling files. **Project inventory is a union; authorization is per file.**

### An implementable scope model

Introduce a `ProjectScope` that owns a normalized, non-overlapping set of
admitted roots/mounts and a `PolicyContext` for each selected file:

```rust
/// Files that can be indexed for this invocation; not a blanket capability.
struct ProjectScope { mounts: Vec<AdmittedRoot> }

/// The effective config and allowed roots of one file's checks.
struct PolicyContext {
    config: Arc<Config>,
    allowed: Arc<RootSet>,
    written_link_root: PathBuf,
}
```

`RootSet(file)` starts with the pre-existing invocation selection root for
selected files, then admits the effective policy bases of that file's
governing-ancestor frames. A shared template contributes no root merely
because its *source* is elsewhere; a governing frame imported through it may
contribute one. Normalize/deduplicate roots; one ancestor root subsumes a
descendant in that file's set. `ProjectScope.mounts` is the union of the
selected files' root sets and any roots subsequently required by an actual
dependency demand. It permits storage/discovery; it does **not** authorize a
source file merely because some other selected file contributed a root.

For the fixture: `RootSet(a.md)={repo/docs}`,
`RootSet(b.md)={repo}`; `ProjectScope={repo}`. `a.md` retains old behavior;
`b.md` can resolve `repo/public/logo.svg`; default checked files remain
within `repo/docs`; displayed names stay relative to `repo/docs`. The
selected child and nested configs are still chosen by nearest-file discovery.

### Preflight and fixed point

1. **Discover candidate paths within the fixed selection root.** A cheap
   metadata walk gathers Markdown files and configuration filenames; it does
   not parse documents. Explicit paths are added after canonical path
   normalization. Do not choose the project resolution boundary yet.
2. **Resolve each candidate's nearest configuration.** Group files by
   selected config path, elaborate each distinct config once in its frame
   context, derive its `RootSet`, and validate bound site directories against
   that set. This requires separating `Config::load` from
   `with_sites_inside(root)`: workspace-dependent site validation must follow
   frame elaboration and scope derivation, not precede it. Store the resulting
   `PolicyContext` per file (or per config+invocation-root key).
3. **Monotone scope closure.** Union the selected contexts' roots into
   `ProjectScope`. An explicit path outside the selection root is provisionally
   held until a discovered selected config admits it; when admitted, load its
   nearest config and add its context. For this uncommon explicit-path case,
   a config-only preflight of the selection subtree may be needed even if a
   nested config currently governs no Markdown file. Repeat until neither
   the set of admitted roots nor the set of demanded candidate/config pairs
   changes. The process terminates because inputs and config files are finite
   and each pair/root is processed at most once.
4. **Build the index only after selected scope stabilizes.** A single-file
   rule may inspect a specifically named in-scope target from disk without
   walking all mounts. If selected rules need a complete cross-file index,
   walk the admitted mounts, deduplicate by absolute path, and load each
   dependency file's own config for its kind/domain. This needs a
   `ConfigResolver` whose nearest-config search is bounded by that file's
   admitted mount, rather than the old `Workspace::config_for` which rejects
   anything outside the invocation root and reuses the invocation config at
   its root. Merely discovering an
   unrelated sibling config in this dependency walk must **not** recursively
   admit its external roots for every selected file. Additional roots enter
   the worklist only when evaluating that file as a selected source or when a
   concrete dependency demand requires its outgoing policy. This makes the
   closure demand-driven rather than an uncontrolled scan of the config
   neighborhood.

For ordinary default `check`, all candidates are known after the one fixed
selection-subtree walk, so step 3 normally converges in one pass. A long
`extend` chain is traversed within each config elaboration (bounded by the
existing 128-unit limit), not by repeated whole-workspace rescans. This is
the intended fast path.

### Authorization must survive into the index

`WorkspaceIndex` may contain files from `repo` because of `b.md`, but a
lookup originating from `a.md` must still use `RootSet(a.md)`, not the index's
global union. Pass the source's `PolicyContext` (or an interned scope ID) to
`resolve_link`, `local_link_targets`, and `local_target_status`. Check both
lexical containment and canonical containment of the *candidate* under an
allowed root before using inventory/file existence; outward symlinks remain
forbidden. Filter site-route candidates with the same source scope. A target
present in the global index but not admitted for this source is
`OutsideWorkspace`/incomplete, never silently `File` or `Missing`.

Cross-file rules need the same discipline. For a selected source `f`, form
its candidate set from indexed files inside `RootSet(f)` and its configured
domain, then report diagnostics attributable to `f`. Do not compute one
workspace-wide default-domain pair set and subsequently filter only the
output; that can contaminate `a.md` with parent/sibling comparisons which its
config never admitted. Rules with symmetric pairs may use both directions
internally, but authorization and reporting remain source-relative.
`written_link_root` stays the invocation selection root for selected files;
external dependency files can use the root of their own selected context.

### Complexity and test gates

Let `N` be candidate paths in the selection subtree, `C` distinct effective
config contexts among them, `E` total loaded `extend` edges, `M` distinct
admitted mounts, and `I` dependency files actually indexed. With directory
and compiled-config caches, preflight should cost approximately
`O(N + E + C log M)` metadata/merge work plus policy matching, not
`O(N × E)` reloads. A complete cross-file index adds `O(I)` file discovery
and parsing, with each admitted root walked at most once; the number of
fixed-point iterations is bounded by newly admitted roots or demanded
config-path pairs. For a link candidate, an interned root set or prefix trie
gives `O(depth)` membership, with canonical containment checked only for
targets actually inspected. These are design targets, not measured results.

Required regression fixtures:

* The standalone `docs` plus nested `sub` counterexample above: from `docs`,
  `b.md` resolves parent/public and sibling targets, but `a.md` does not gain
  access. Neither file's old displayed name changes; bare `check` does not
  select parent documents.
* A second nested config in another child subtree extending a disjoint
  template/governing root: mounts are a bounded union; neither source gains
  the other's mount; the filesystem least-common ancestor is not walked.
* An outward symlink from an admitted public directory remains unauthorized
  despite the target being listed in a wider index.
* Instrument config reads, glob compiles, directory visits, and peak memory
  for 10k files / many nested configs / repeated templates, comparing cold
  and warm runs to the no-frame baseline. Verify output and authorization
  before accepting any speed claim.
