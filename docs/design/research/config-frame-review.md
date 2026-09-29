# Independent review: configuration environment frames

Reviewed 2026-09-29 against the in-progress `.temp/config-frames` worktree. This is a source and regression-contract review, not a claim that the branch is finished. The author is still editing, so findings distinguish a demonstrated behavior from pending implementation work.

## Resolved during review — nested frame widens the dependency index without borrowing authorization

The initial project-root implementation fixed the index scope from the invocation-selected config chain. A deeper selected file could import a governing ancestor and its site/public targets, but the index never walked those targets. `with_sites_inside` did not reject the case; the link silently appeared absent. The implementation now preflights configs of selected files and widens the index root only to explicitly admitted ancestors (`src/workspace.rs:261-277`), then walks dependencies if needed (`:281-312`). Per-source authorization in `src/index/mod.rs` and `src/rules/links.rs` prevents a standalone neighbor from borrowing another file's wider grant. `src/rules/cross_file.rs` likewise uses the invocation-root union for cross-file comparisons, preserving root-invocation compatibility.

**Minimal differential reproduction:** Under `.temp/review-nested`, root `seiso.toml` has `[lint] select=["LNK001"]` and `[[sites]] path="docs/sub/**", root=".", public="assets"`; `docs/seiso.toml` is standalone; `docs/sub/seiso.toml` extends `../../seiso.toml`; `assets/logo.png` exists; `docs/sub/page.md` links `/logo.png`. Before the preflight, running `seiso check sub/page.md --output-format json` from `docs` exited 1 with LNK001, `Local link target "/logo.png" does not exist in the workspace.` Temporarily making `docs/seiso.toml` extend root produced `[]`, exit 0, isolating the missing nested expansion. After the preflight change, the original standalone fixture also produces `[]`, exit 0. The fixture remains in its standalone state. `tests/config_frames.rs` now covers the nested import and standalone-neighbor denial.

**Final concern status:** No open defect from this reproducer. The approach deliberately supports nested ancestor-chain expansion; it does not implement arbitrary disjoint project mounts, which the current design excludes.

## Measured long-tail behavior and residual optimization choice

**Current evidence:** `ConfigResolver` memoizes ancestor selection including negative results and reuses compiled configurations by selected normal-discovery source. `docs/design/research/config-frame-performance.md` records an equivalent-source A/B comparison with raw logs in project-local `.temp`: 5,000 misses against 256 kind mappings took baseline 101.57 ms versus current 100.76 ms median; 10,000 distinct path checks took 222.26 versus 221.47 ms. There is no material matching regression on these tested tails. A 128-unit `seiso.toml` chain was 18% slower (about 6.7 ms/load), attributable to a priority probe needed for correct governing-edge classification; the equivalent `.seiso.toml` chain did not regress. The compiled `Config` is still cloned per file, but these data do not establish a material end-to-end cloning cost.

**Judgment:** The extreme-chain slowdown is a measured, bounded trade-off for correct precedence, not a defect worth sacrificing semantics to remove. The current source cache and single-base matching fast path are reasonable; there is no evidence to justify a more complex `GlobSet` design now.

**Optional next measurement:** A full-CLI fixture with actual Markdown files, project dependencies, and JSON output would establish whether per-file `Config` cloning is material. If it is, share compiled policies through `Arc<Config>` or an arena; otherwise keep the simpler ownership model. Preserve context in cache keys if future reuse expands to shared templates: a template source alone is not its semantic identity.

## Observations, not findings

* The `FramedValue` overlay stores frame IDs on individual TOML nodes, so replacement of a `lint.per-file-ignores` key updates its value and frame together. The mapping extension arrays are appended after effective base arrays; reverse scanning then gives later extension entries precedence. The focused tests exercise both behaviors.
* `environment_report` adds a top-level `environments` object to `policy` while retaining `configurations`' `Settings` field shapes. This is an additive JSON change; compatibility with consumers that reject unknown top-level fields is not demonstrated, but there is no evidence of such an external contract here, so I do not elevate it to a defect.
* The same source label is still used for some final validation errors even when a malformed surviving declaration came from an ancestor; precise origin diagnostics are desirable but not necessary to establish the core semantic invariant.
* `ConfigResolver` caches nearest selected config by directory and compiled normal-discovery config by selected source. The selected config always determines its own normal base; templates are elaborated only inside that selected config, so no template is incorrectly cached by source alone. The cache belongs to a `Workspace` instance, and the CLI creates one per invocation. A future long-lived API or daemon would need invalidation for on-disk changes; no current CLI regression is demonstrated.
* Site containment checks lexical resolved directories at configuration load and link target status checks the nearest existing path under the **source-specific** allowed root. The scoped-index tests include a symlink escape, and the selected-file preflight test establishes that a standalone neighbor cannot borrow another nested config's root grant. I found no concrete route that reads an out-of-scope symlink target in this implementation.
* The docs distinguish selection, invocation-project, global index, per-source authorized, and written-`/` roots. The architecture reference explicitly limits this implementation to ancestor-chain expansion, not arbitrary disjoint project mounts. Such a disjoint `extend` is treated as a shared template, so its mere file location does not grant a new filesystem project.

## Final assessment on the stabilized tree

No substantive open correctness or compatibility finding remains from this pass. The nested-project issue above was demonstrated and fixed; the root-invocation standalone-config cross-file compatibility case has a dedicated passing regression test. The focused frame suite was reported passing 24/24 after these changes, and the exact nested CLI reproduction now outputs `[]` with exit 0. This is not a claim that every command and platform has been revalidated independently; full-suite, formatting, and release-gate results belong in the integration record.

## Review limits

I read the frame model and performance notes first, then `src/config/frame.rs`, the modified `src/config/mod.rs`, `src/workspace.rs`, `src/commands.rs`, and `tests/config_frames.rs`. I did not independently run the full test suite or compare release timings during concurrent edits. Findings should be revisited only if these cited paths change materially.
