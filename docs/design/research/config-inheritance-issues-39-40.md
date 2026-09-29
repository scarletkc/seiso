---
kind: plan
---

# Configuration inheritance review: issues #39 and #40

Reviewed on 2026-09-29 against `origin/main` at `3fb1baa` (the local `main`
checkout was 14 commits behind and had unrelated uncommitted files). This is a
pre-implementation design and code-path review, not a runtime regression test
or current implementation guidance. Recheck its recommendations against the
frame-based implementation and its compatibility tests before adopting them.

## Scope and causal chain

- [#40](https://github.com/scarletkc/seiso/issues/40) is the prerequisite bug:
  `load_extended` merges TOML values before `Config::from_value` compiles path
  patterns; the merged result no longer knows which configuration supplied
  each entry. `Config::relative_path` then uses only the selected configuration's
  directory. Inherited `include`, `exclude`, kinds, domains, sites, and
  per-file ignores therefore move when a child extends a governing ancestor.
- [#39](https://github.com/scarletkc/seiso/issues/39) is the additive-list and
  initialization feature. `init` currently rejects any discovered configuration.
  The existing overlay replaces ordinary arrays. The `extend-exclude` precedent
  from [PR #38](https://github.com/scarletkc/seiso/pull/38) accumulates an
  extension array and applies it after the effective base array. The proposed
  `extend-kinds`, `extend-sites`, and `extend-domains` can follow this rule.
- Implement #40 first. Merely appending mappings in #39 cannot preserve a
  parent's mappings or site routes if their patterns are still evaluated from
  the child's directory.

Relevant code in `origin/main`: `src/config/mod.rs` lines 203-276, 301-397,
460-509, 712-807; `src/commands.rs` lines 510-659; `src/workspace.rs` lines
138-402 and 516-535. The current reference explicitly documents the old
selected-configuration-relative behavior.

## Design invariant and representation

For a fixed workspace root and file `f` below the child, installing an empty
child configuration that extends its governing parent should leave the
effective file policy unchanged. A shared template must retain the existing
rebasing behavior. These are two distinct semantic relationships, not two glob
syntaxes. Classify each `extend` link when loading it; bind each surviving
path-bearing value to its effective base directory; then merge lists and maps.
Match absolute file paths against the bound base rather than rewriting glob
strings. Site `root` and `public` need the same bound base as site `path`.

Keep the public `Settings` serialization shape stable. In particular, do not
replace existing JSON string arrays with origin objects. `policy` can add a
separate, deterministic provenance section containing base directories aligned
with effective entries. This also makes inherited behavior inspectable, as
requested by #40. A template's source file and its effective base directory
must be reported separately.

## Material acceptance gap: workspace root versus inherited site roots

`Workspace::discover` currently sets the workspace root to the nearest
configuration directory. `with_sites_inside` rejects a site `root` or `public`
outside that root. Suppose repository `seiso.toml` has `[[sites]] path="docs/**"
root="."`, and `docs/seiso.toml` extends it. Once #40 correctly preserves the
parent base, `root="."` denotes the repository root; running from `docs/`
selects `docs/` as the workspace root and rejects the inherited site. A parent
`public` directory that is a sibling of `docs/` has the same problem. This is
deduced from the existing root and containment code, not observed from a
patched implementation. Decide whether extending a governing ancestor keeps
the ancestor workspace root, whether such site paths are intentionally invalid
from a child invocation, or whether containment can safely be broadened. The
current #40 criterion (preserve every inherited site for every file below the
child) cannot unconditionally coexist with the current root rule.

The fixed-root invariant above is narrower than #39's unqualified statement
that creating a child changes no existing policy. Even before the proposed
changes, invoking the CLI from the child can change workspace-relative links,
file discovery, and cross-file comparison scope because the workspace root
changes. Specify the invocation/root in the criterion or resolve root semantics
first. Do not silently alter established workspace-root behavior without a
compatibility decision and release note.

## Focused regression matrix

1. Parent config in repository root; child in `docs/`: parent `include`,
   `exclude`, kinds (including `generated`), domains, per-file ignores, site
   `path`, `root`, and `public` each retain their physical referent. Check both
   `policy` and representative diagnostics, not only deserialized settings.
2. Three-level chain: a shared template in a subdirectory extends into a
   governing parent, then into a child. Template paths bind to the parent's
   directory; parent paths remain parent-bound; child paths are child-bound.
3. A child extends a non-governing file in an ancestor directory, including a
   lower-priority same-directory config, and a template below the child. Both
   retain the existing selected-config-relative result.
4. Test `--config` separately; its selected file remains rooted at the
   workspace root. Do not conflate this with ordinary discovery.
5. `extend-*` mapping lists accumulate in chain order after the effective
   (possibly replaced) base list. Verify last-match-wins, malformed entries,
   empty/missing fields, and policy JSON. Specify whether a child's plain
   `[[kinds]]` can override an inherited `[[extend-kinds]]`; under the proposed
   ordering, it cannot unless the child also uses `[[extend-kinds]]`.
6. Compare policies before and after `init --extend` for every pre-existing
   file in a fixture, allowing only explicitly suggested changes. Test root,
   child, existing-file refusal, and parent `pyproject.toml`/`.seiso.toml`
   precedence; retain atomic, no-clobber creation.

## External signals and applicability

- [Git's `.gitignore` documentation](https://git-scm.com/docs/gitignore)
  demonstrates a production-proven per-entry origin model: patterns from
  different ancestor files match relative to their own directories. Its exact
  precedence is not seiso's design, but retaining origin is relevant.
- [Ruff's configuration documentation](https://docs.astral.sh/ruff/configuration/)
  uses closest-file discovery and explicit `extend`; its inherited relative
  paths are based on the selected project's root, which is useful for shared
  templates. Seiso's proposed governing-parent/template split is deliberately
  different and must be explicit, not accidental.
- [Shan et al., *ConfLogger*, ICSE 2026](https://ink.library.smu.edu.sg/sis_research/10743/)
  studies how exposing configuration information improves diagnosis. This
  supports adding clear provenance to `policy`, but does **not** validate
  seiso's specific inheritance semantics or justify adopting its ML machinery.

## Recommendation

Amend #40 with an explicit workspace-root/site-containment decision and a
before/after differential test. Then implement #40 with bound path provenance
and additive policy output; implement #39 on that foundation. Keep the two
changes as separate reviewable PRs because #40 changes established behavior
while #39 adds configuration syntax and a new command mode.
