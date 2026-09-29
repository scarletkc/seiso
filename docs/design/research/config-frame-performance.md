# Configuration-frame performance: long-tail baseline and design constraints

This note belongs to the frame-based inheritance implementation. It records
an opt-in release-mode diagnostic at `tests/config_frame_perf.rs`, an
equivalent-source alternating baseline/current comparison, and the limits of
what the measurements establish.

## Real workload and failure modes

The current repository has 60 Markdown files in 11 distinct directories, with
at most three relative directory components (`rg --files -g '*.md' -g
'*.markdown'`, then unique parents, 2026-09-29). In that ordinary shape,
workspace I/O and Markdown analysis may dominate; elaborate pattern indexing
would be premature. The meaningful tails are (a) many distinct leaf directories
in a monorepo, (b) deep explicit `extend` chains up to the current 128-file
limit, (c) hundreds of mappings with last-match-wins semantics, and (d) many
child configs sharing one ancestor/template. These are separate axes, not one
synthetic super-workload.

`workspace::load` already caches policy selection by **exact file directory**
within one invocation. Thus repeated files in one directory do not repeatedly
call `Workspace::config_for`; distinct directories still do. On a miss,
`config_for` checks ancestors and `config_in` probes three filenames with
`fs::metadata` per ancestor, stopping at the workspace root. If it selects a
non-root configuration, `Config::load` reads, parses, overlays, validates, and
compiles its whole inheritance chain. Compiled mapping matchers are scanned in
reverse order. These source-level mechanisms predict roughly:

| Path | Tail cost before a frame-aware cache | Relevant variable |
| --- | --- | --- |
| Directory selection | `O(U·D)` filesystem probes, at most three per ancestor | `U` distinct file directories, depth `D` |
| Child config evaluation | `O(K·(C+P))` parse/overlay/compile work | `K` distinct selected child configs, chain length `C`, effective pattern count `P` |
| Mapping miss | `O(F·P)` matcher tests | `F` selected files; reverse scan cannot stop on a miss |

The cost of each operation depends strongly on storage, antivirus, filesystem
cache, and pattern shape. These formulae describe work counts, not latency
predictions. `Config::load` can also call `fs::canonicalize` for each inherited
file; its cycle check scans the at-most-128-element stack. The latter is
bounded and is not the first optimization target.

## Reproducible baseline

Run from this worktree:

```powershell
cargo test --release --test config_frame_perf -- --ignored --nocapture
```

The ignored test creates and removes unique fixtures under this worktree's
`.temp`; it does no network I/O. It measures three sequential wall-clock runs
per scenario after the binary is built. It uses `black_box` to retain the
computed result. The fixture represents chains of 1/8/64/128 config files,
0/64/256 kind mappings, and absent child configs at depths 1/8/32. Pattern
matching uses a nonmatching file so all mapping entries are considered. It
also measures 32 selected child configs sharing a 64-pattern template, which
distinguishes reusable parsing from context-dependent binding.

Environment: Windows 11 10.0.26200, Intel Core i9-12900H (20 logical CPUs),
Rust/Cargo 1.98.1, `cargo test --release`, commit `3fb1baa` plus concurrent
unrelated command work. The first run compiled before `src/config/mod.rs` was
modified for frames. All values below are **total elapsed milliseconds** for
the stated operation count; min–max is three repetitions, not a confidence
interval.

| Scenario | Operations | First-run range, ms | Per-operation implication |
| --- | ---: | ---: | ---: |
| Load chain length 1 | 30 | 9.1–11.7 | 0.30–0.39 ms/load |
| Load chain length 8 | 30 | 68.5–79.8 | 2.28–2.66 ms/load |
| Load chain length 64 | 30 | 452–962 | 15.1–32.1 ms/load; noisy |
| Load chain length 128 | 30 | 1179–1217 | 39.3–40.6 ms/load |
| Load 0 kind patterns | 30 | 5.4–6.4 | 0.18–0.21 ms/load |
| Load 64 kind patterns | 30 | 42.9–44.6 | 1.43–1.49 ms/load |
| Load 256 kind patterns | 30 | 137–161 | 4.57–5.38 ms/load |
| Match kind miss, 0 patterns | 5,000 | 6.30–6.30 | 1.26 µs/file |
| Match kind miss, 64 patterns | 5,000 | 29.6–30.1 | 5.92–6.02 µs/file |
| Match kind miss, 256 patterns | 5,000 | 98.8–101.0 | 19.8–20.2 µs/file |
| Discover absent config, depth 1 | 1,000 | 99.2–100.8 | 0.099–0.101 ms/call |
| Discover absent config, depth 8 | 1,000 | 794.8–796.3 | 0.795–0.796 ms/call |
| Discover absent config, depth 32 | 1,000 | 3762–6761 | 3.76–6.76 ms/call; noisy |

The repeated-discovery scenarios deliberately call the public API without
`workspace::load`'s exact-directory cache. They isolate cache-miss cost, not
end-to-end cost for 1,000 files in one directory. A later run of the added
128-distinct-leaf test gave 25.8–27.4 ms total with two missing-config levels;
that run overlapped frame edits and machine contention, so it is a **workload
shape check**, not a comparable baseline or optimization result. The later
run showed about 2× slower values even for unchanged cases, confirming that
cross-run ratios are unsafe without a stable tree and quiet machine.

## Equivalent-source alternating comparison

To reduce source and scheduler confounding, the final diagnostic used the
**same** `tests/config_frame_perf.rs` source in two worktrees: a detached
`3fb1baa1d4dc8f066e80e99d89747b1fdfa9b5db` baseline at
`D:/Code/seiso/.temp/perf-baseline`, and the frame implementation in
`D:/Code/seiso/.temp/config-frames` (same HEAD plus uncommitted implementation).
Both used the same Cargo.lock, Rust toolchain, machine, release profile, and
fixture generator. Their baseline/current binaries ran sequentially in
**B-A-B-A** order; each run has three repetitions. Build time was excluded.
For traceability, the current `src/config/mod.rs` SHA-256 at comparison was
`734372d01bfbf0c48249e2ec0a8a02bc0036706ffacbbd287fa7bf8a12def698`
and `src/config/frame.rs` was
`c1e16b5aa72aed85e8bdc435c9b126af8d32f8712dec45b5dcf4540885e89b45`.
The final test source SHA-256 (after adding the 10k-path scenario) was
`1b56cf30e79d04288cd3fc64fdb0314d1922a6ce6a318e7a`; that scenario was
copied identically to both worktrees and run B-A once more. The preceding
B-A-B-A source was also identical between worktrees but lacked that scenario.

Raw logs remain under the two worktrees' `.temp` directories:
`frame-benchmark-baseline-{a,b}.txt`,
`frame-benchmark-current-{a,b}.txt`, and
`frame-benchmark-{baseline,current}-10k.txt` (current logs in the current
worktree and baseline logs in the detached baseline worktree).

| Scenario | Operations | Baseline median [range], ms | Current median [range], ms | Interpretation |
| --- | ---: | ---: | ---: | --- |
| Kind miss, 64 mappings | 5,000 | 29.59 [28.93, 30.45] | 29.39 [28.26, 30.37] | No material difference |
| Kind miss, 256 mappings | 5,000 | 101.57 [99.64, 108.84] | 100.76 [99.24, 104.36] | No material difference |
| 10,000 distinct candidate paths, include + kind with 256 mappings | 10,000 | 222.26 [219.64, 228.37] | 221.47 [215.95, 222.07] | No material difference; path-only, not full CLI |
| Load 32 child configs sharing 64-pattern template | 32 | 42.72 [39.14, 45.46] | 40.01 [38.14, 43.09] | Similar source reuse tail |
| Load 128-file `seiso.toml` chain | 30 | 1118.57 [1085.83, 1174.36] | 1320.33 [1234.99, 1381.11] | Current is 18.0% slower in this extreme one-shot chain |
| Load identical 128-file `.seiso.toml` chain | 30 | 1119.67 [1079.96, 1160.28] | 1091.00 [1029.14, 1150.73] | No regression when governing-edge priority needs no metadata probe |

The 10k-path scenario uses prebuilt `PathBuf`s and invokes `includes` and
`kind_for`; it does not create 10,000 Markdown files or measure parser/JSON
costs. Six-sample medians are descriptive, not confidence intervals. The
near-identical 64/256-pattern timings are more credible than the earlier
contention-overlapped 1.6–7× apparent regression. They justify retaining the
simple one-frame direct matcher path and **not** adding `GlobSet` indexing.

The 128-chain difference is explained by a necessary semantic check:
`governing_ancestor` must test whether a higher-priority `.seiso.toml` exists
before classifying an inherited `seiso.toml` as a governing ancestor. The
matched `.seiso.toml` variant has the same 128 depths and TOML content but
skips that metadata probe, and its regression disappears. This is strong
causal evidence for the probe, though it is still an inference rather than a
syscall trace. The chain limit is 128 and `Workspace` now caches compiled
selected configs within an invocation, so this cost is paid once per distinct
selected config, not per file. A direct API client repeatedly calling
`Config::load` still pays it each time. The ~6.7 ms/load difference at the
128-depth median is acceptable for the correctness guarantee on this bounded,
unusual chain; do not remove the precedence check merely to win a synthetic
benchmark.

Resolver results need more care: 128 distinct directories with a 256-pattern
root policy took 48.1–53.2 ms on the baseline. Current first repetitions
took 37.4–39.9 ms, while later repetitions of the **same `Workspace`** took
17.6–20.0 ms because its new invocation-scoped resolver cache was warm. The
warm figure is not a fair single-invocation speedup for 128 previously unseen
directories. It demonstrates amortization for repeated resolution, not a
guaranteed improvement for a one-pass CLI walk.

## Frame-aware optimization design

1. **Retain the frame as semantic identity, not as per-pattern filesystem
   work.** A pattern should store a small frame identifier. Frames should own
   normalized bases. For each candidate file and each *distinct effective
   base*, derive the relative path once and reuse it across include, exclude,
   kinds, domains, sites, and per-file ignores. Hundreds of patterns may share
   one frame. Never recalculate or normalize the absolute path for each entry.
   A governing ancestor and a template can share the same effective base;
   grouping by base is safe for matching, while keeping separate frame IDs is
   necessary for provenance.
2. **Memoize directory discovery within a workspace evaluation, including
   negative results.** A directory-to-nearest-config cache should be populated
   along an ancestor walk, not only for the original leaf directory. This
   changes many sibling leaves from repeated ancestor probes to mostly local
   probes. The cache belongs to one invocation; a process-global cache would
   make later invocations miss newly created/deleted configs. Preserve
   `.seiso.toml > seiso.toml > pyproject.toml` and the latter's `[tool.seiso]`
   check, including malformed/unreadable-file errors.
3. **Separate parsed-source reuse from evaluated-frame reuse.** A parsed TOML
   unit is keyed by canonical source path within an invocation. A bound
   configuration is *not* keyed by source path alone: the same shared template
   can be evaluated under different `policy_base` values, and `load_from`
   supplies a distinct selected base. A safe evaluated key must include the
   selected path and evaluation context (effective base, explicit/discovered
   mode, and any ancestor relationship affecting frames). Parse once per
   source where useful, but bind each context correctly. Keep cycle detection
   on the active recursion stack even when a parsed unit is cached.
4. **Consider grouped multi-pattern matching only after profiling.** `globset`
   provides `GlobSet` and candidate preprocessing for many patterns in one
   pass. For last-match-wins lists, preserve original entry order and select
   the largest matching index, including matches from different bases. A
   `GlobSet` per base/type adds build time and memory and could regress common
   two-pattern configs. Benchmark a threshold (e.g., dozens of entries) before
   adopting it; do not change glob syntax or precedence while optimizing.

The first two steps attack measured work multipliers without changing the
configuration model. The third is essential if shared templates make source
reuse attractive. The fourth is optional until the full `workspace::load`
profile shows mapping scans matter in actual large workspaces.

## Measurement gate for the implementation

The equivalent-source comparison establishes that the one-frame/large-pattern
tail does not regress, and the workspace resolver amortizes repeated
selection. It does **not** claim an end-to-end CLI speedup over the old
version: the new child-project behavior has no equivalent old-version
workflow. The following section measures the current CLI with actual
Markdown files and a root-invocation comparator. Correctness gates must cover nearest-config
precedence, template rebasing, governing-ancestor binding, `--config`,
last-match-wins, and source/provenance output. A microbenchmark gain is not
enough if the CLI walk, Markdown parser, or JSON output dominates latency.

## Child invocation with project-wide dependencies

The later project-root/selection-root split changes the workload itself: a
child invocation can now check an inherited parent project's sibling file,
but should not scan that project for a single-file-only rule. The ignored
`child_project_dependency_costs` test in `tests/config_frame_perf.rs` exercises
this distinction through `workspace::load(..., LoadScope::Check)` followed by
`analysis::check`:

```powershell
cargo test --release --test config_frame_perf -- --ignored --exact child_project_dependency_costs --nocapture
```

Its fixture has `root/seiso.toml`, `root/docs/seiso.toml` extending it, and
`root/docs/topic/seiso.toml` extending the middle unit. From `docs/topic`, it
requests only `guide.md`, which links a sibling reference page. The project
contains 1,024 **distinct** Markdown reference documents in 32 sibling
directories. A second fixture adds eight sibling configurations extending the
root. `KND001` is a single-file rule; preview-enabled `LNK002` requires a
cross-file index. The test creates fixtures outside timing and repeats each
case once with an initially empty parse cache and once with that cache warm.
"Cold" refers only to seiso's parse cache, not the OS page cache. The `0`
and `8` cases were generated afresh twice in release mode; raw outputs are
`.temp/frame-child-project-current-{b,c}.txt` in the current worktree. An
earlier `-a.txt` run preceded the final index integration and is provisional.
The measured tree was still based on HEAD `3fb1baa` with uncommitted changes;
SHA-256 fingerprints after the timed runs were `0102126a...4e38585`
(`src/workspace.rs`), `2335f45f...979bd32f` (`src/config/mod.rs`),
`23a28e8a...68fbf0e` (`src/index/mod.rs`), and `afca0cf7...928a30`
(`tests/config_frame_perf.rs`). Full fingerprints can be regenerated from
the retained worktree; these prefixes distinguish this stage from the earlier
matching-only comparison.

| Selection and launch directory | Indexed/selected docs | Load time, release-mode observations | Analysis time | Interpretation |
| --- | ---: | ---: | ---: | --- |
| `KND001` from child | 1 / 1 | cold 4.5–8.9 ms; warm 2.4–5.9 ms | 0.01–0.06 ms | No parent dependency walk |
| `KND001` from project root | 1 / 1 | 45–88 ms | 0.02–0.04 ms | Root invocation walks its full default tree before retaining one selected file |
| `LNK002` from child | 1,025 / 1 | cold 0.90–1.55 s; warm usually 0.18–0.21 s, one 0.80 s outlier | <0.002 s | Parent dependencies loaded but not selected for diagnostics |
| `LNK002` from project root | 1,025 / 1 | cold 0.68–1.23 s; warm 0.13–0.19 s | <0.002 s | Same project-sized dependency set |
| `LNK002` from child, `no_cache = true` | 1,025 / 1 | 0.097–0.155 s after OS warming | <0.002 s | No parse-cache reads or writes; not a cold comparison |

The eight nested sibling configs produced no consistent additional cost
relative to the zero-nested case at this scale. The more important distinction
is semantic: `KND001` does not promote one selected child document into a
project-wide parse; `LNK002` does, and the reported selection remains one
document. These are not speedup ratios against `origin/main`, which could not
perform the same inherited-parent cross-file check. The root-launch case is
the meaningful current-implementation comparator for an equivalent dependency
set. The `LNK002` selected link resolved successfully (zero diagnostics),
whereas `KND001` reports one expected missing-kind diagnostic.

Cache files are written under the **invocation/selection root**, not the
inherited project root. The child-only single-file run created one
`docs/topic/.seiso_cache/*.cache` file and none at the root; its cross-file
run created 1,025 child cache entries. Running the same project check from
the root then created another 1,025 entries in `root/.seiso_cache`. Each
cache contained 248,291 bytes of file payload across 1,025 tiny files;
filesystem allocation and directory metadata add unmeasured storage cost.
`no_cache = true` did not change entry counts. These observations match the
source construction `ParseCache::new(selection_root.join(".seiso_cache"), ...)`
and show that frequent root/child switching duplicates an otherwise
content-addressed cache. A shared project cache is a possible future
experiment, not an automatic fix: the parent directory may be read-only and
users may rely on the existing location for cleanup and ignore rules.

For a process-level probe, the retained fixture
`.temp/frame-child-cli-fixture` has the same 1,025-document shape without
the eight nested configs. The current release binary ran `seiso check` from
the child or root with `--preview --select KND001|LNK002 --output-format json
--exit-zero`; output was redirected to `.temp`, outside the fixture. A
PowerShell `Start-Process -WindowStyle Hidden` loop sampled
`Process.PeakWorkingSet64` every 5 ms, while a stopwatch measured process
startup through exit. Raw summary is `.temp/frame-child-cli-metrics.csv`;
individual JSON and stderr files are adjacent. Sampling and process startup
add uncertainty; memory figures are approximate process high-water marks, not
isolated live heap sizes.

| CLI case | Wall time | Sampled peak working set | Scope |
| --- | ---: | ---: | --- |
| Child `KND001`, cold then warm | 100 / 56 ms | 8.5 / 6.8 MiB | One selected document |
| Child `LNK002`, first cold | 561 ms | 19.9 MiB | 1,025 indexed documents |
| Child `LNK002`, four warm runs | 136–176 ms (median 143 ms) | 17.9–22.6 MiB | Same dependency set |
| Root `LNK002`, first cold | 650 ms | 23.4 MiB | Same dependency set |
| Root `LNK002`, four warm runs | 137–163 ms (median 147 ms) | 17.8–22.5 MiB | Same dependency set |
| Child `LNK002 --no-cache`, four runs after OS warming | 118–136 ms (median 125 ms) | 23.5–24.2 MiB | Parses tiny documents without disk-cache I/O |

There is also an equivalent **old-version root-invocation** control. The
detached `3fb1baa` binary and current binary alternated four times from the
same project-root fixture with `check docs/topic/guide.md --preview --select
LNK002 --no-cache --output-format json --exit-zero`. Both returned code 0 and
the same three-byte empty JSON diagnostics array. Baseline wall times were
119–198 ms (median 132.5 ms), current 118–134 ms (median 119 ms); sampled
peak working-set ranges were 20.4–23.4 MiB and 18.8–24.0 MiB,
respectively. With only four pairs, process-startup noise, and overlapping
ranges, this establishes **no demonstrated end-to-end regression** on the
equivalent root workflow, not a proven 10% speedup. Raw alternating results
are `.temp/frame-child-root-baseline-comparison.csv`, with each run's JSON and
stderr beside it. The old binary has no equivalent child-invocation project
scope, so its root run cannot be used as a direct speed baseline for that new
feature.

For this deliberately tiny ~40-byte/document corpus, `--no-cache` is
slightly faster than warm cached reads. That is a workload-specific **negative
result** for assuming a parse cache always helps; it does not justify a
length threshold without a second fixture of realistic 1–10 KiB documents.
The practical acceptance judgment is narrower and positive: the new frame
scope does not impose a project-wide parse on single-file checks, and
cross-file checks pay for the project index only when their selected rules
require it. Further optimization of tiny-document cache reads or cache
placement should be driven by real user traces, not this synthetic corpus.

## External grounding

- [Git's `.gitignore` documentation](https://git-scm.com/docs/gitignore)
  confirms that a production system retains each pattern's source-directory
  interpretation. This supports frame identity but not seiso's exact `extend`
  classification.
- [`globset::GlobSet` documentation](https://docs.rs/globset/latest/globset/struct.GlobSet.html)
  documents one-pass multi-glob matching and candidate preprocessing; it is a
  possible implementation instrument, not evidence of a seiso speedup.
- [Rust compiler incremental-query guide](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)
  explains how dependency-aware reuse requires tracking the inputs of a
  query. The relevant lesson here is the contextual cache key, not adopting a
  compiler-scale query engine for a short-lived CLI.
- Hammer et al., [*Adapton: Composable, Demand-Driven Incremental Computation*,
  PLDI 2014](https://matthewhammer.org/adapton/) formalizes context-sensitive,
  demand-driven reuse. It motivates an experiment for repeated evaluations of
  shared configs, but its framework is not directly validated for seiso's
  small, bounded inheritance chains.
