---
kind: howto
---

# Development

The [architecture reference](../reference/architecture.md) maps module responsibilities and
command execution. Use this guide to build, test, inspect parser behavior, and
run performance or ecosystem comparisons.

## Build and validate

Use a stable Rust toolchain with Cargo, rustfmt, and Clippy.

```sh
cargo build --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests include source spans in English, Chinese, and Japanese; Markdown
structure; configuration precedence and inheritance; diagnostic snapshots;
and repeatable CLI output. Snapshot changes are reviewed as ordinary Git
diffs. `INSTA_UPDATE=always cargo test` regenerates snapshots
when an output change is intentional.

With Python 3.12 or later, check packaging and corpus tools:

```sh
python -m unittest discover -s scripts/tests -p 'test_*.py'
python -m unittest discover -s corpus -p 'test_*.py'
python -m unittest discover -s corpus/evaluation -p 'test_*.py'
```

Run `seiso rule --all` to read the implemented rules. Their explanations are
embedded from the Markdown files in `docs/rules/`. Link from them to other
repository files with relative links; `seiso rule` points those links at the
release tag of the running version.
Their positive and negative Markdown examples execute as tests; positive
diagnostics are stored in snapshots. Add regression cases for fragment
boundaries, languages, source mappings, and suppression scope when changing
a rule. Use the [checking guide](checking.md) to exercise the CLI.

## Maintenance scripts

The `scripts/` package groups tools by responsibility:

| Directory | Purpose |
| --- | --- |
| `ci/` | CI scope selection and pull request title validation |
| `release/` | Versioning, packaging, publication, and installation checks |
| `evaluation/` | Corpus runs, milestone evaluation, summaries, and benchmarks |
| `tests/` | Python tests for the maintenance tools |

Run tools as Python modules from the repository root, for example
`python -m scripts.release.release check`. The [publishing guide](publishing.md)
and [evaluation procedure](../../corpus/docs/evaluation.md) document their commands.

## Inspect documents

`seiso parse` discovers the workspace, respects include/exclude patterns and
Git ignore files, and parses selected Markdown files. Each file uses its
nearest configuration. Paths printed in reports are relative to the workspace
root. JSON is written to stdout; input and filesystem errors are reported
separately and return exit code 2. This command inspects structure; it does
not apply lint rules or return lint exit code 1.

```sh
cargo run -- parse docs/ --output-format json
cargo run -- parse --config seiso.toml README.md
```

`--stdin-filename PATH` reads stdin instead of the file at PATH. PATH must
be inside the workspace and selected by the active configuration. The file
does not need to exist, and its contents are never written to disk.

`parse --output-format json` also includes `section_annotations` for each file.
The [section reference](../reference/sections.md) describes these heuristic
predictions and their source evidence.

The document model records frontmatter errors as content facts. A failed
frontmatter declaration has no effective kind, even if a path mapping exists.
The KND rules turn these facts into lint diagnostics during checking.

## Parser boundaries

The parser accepts CommonMark and GFM with YAML frontmatter. TOML-style
`+++` fences remain ordinary Markdown. Source mappings mark synthesized or
ambiguous decoded text with `exact: false`; consumers must use the containing
source range for those fragments.

Some reference labels with leading whitespace remain plain text in the
upstream parser even when a matching definition exists. For example,
`[the docs][ A  B ]` does not resolve to `[a b]: target.md`. The original
text is preserved, but the link is absent from the link collection. The
`markdown_rs_spaced_reference_limitation_retains_original_text` regression
in [document tests](../../tests/document.rs) records this boundary.

## Milestone acceptance

The [evaluation policy](../evaluation/policy.md) defines promotion and acceptance
requirements. Use the [corpus procedure](corpus.md) for parser evaluation and
the [rule evaluation procedure](../../corpus/docs/evaluation.md) for fresh rule
evidence or historical replay. The [milestone index](../design/roadmap.md#milestone-evidence)
links to dated results. See [publishing](publishing.md) for distribution validation.

## Performance and ecosystem checks

Build a release binary before running the benchmark:

```sh
cargo build --release --locked -p seiso
python -m scripts.evaluation.benchmark_m2 --binary target/release/seiso --output target/performance.json
```

The benchmark generates a deterministic workspace with repeated templates
and similar paragraphs. It verifies cache output equality and measures cold
checks, warm checks, and the warm hook including process startup. Preview
rules are timed separately. Use a native Linux filesystem for local Linux
measurements; the acceptance thresholds apply to the standard Linux CI runner.

To isolate DUP003/OWN002 prefix sorting with mostly distinct prose shingles,
run:

```sh
python -m scripts.evaluation.benchmark_prefix_sort --binary target/release/seiso
```

Pass `--baseline PATH` to compare a release binary with the same package version;
the benchmark checks that their diagnostics match across cold, warm, no-cache,
and selected-file runs.

Cached keys add temporary storage proportional to the shingles in the paragraph
being sorted. To compare whole-process peak memory on Linux, generate the same
200-file fixture in an empty directory and run each same-version release binary
with GNU time (replace both absolute binary paths):

```sh
mkdir prefix-memory
python -c "from pathlib import Path; from scripts.evaluation.benchmark_prefix_sort import fixture; fixture(Path('prefix-memory'), 200)"
cd prefix-memory
/usr/bin/time -v /absolute/path/to/baseline/seiso check --no-cache --select DUP003,OWN002 --output-format json > baseline.json
/usr/bin/time -v /absolute/path/to/current/seiso check --no-cache --select DUP003,OWN002 --output-format json > current.json
cmp baseline.json current.json
```

The duplicate pair deliberately makes `seiso check` exit with status 1; other
nonzero statuses indicate a failed measurement. Compare the `Maximum resident
set size (kbytes)` values over repeated, alternating runs. This measures the
whole process, so small cached-key allocations may be hidden by other memory
use; it does not isolate their exact size or establish a universal memory bound.

The [CI workflow](../../.github/workflows/ci.yml) runs on pull requests targeting
`main` and pushes to `main`. Ordinary documentation changes run only the
repository's document checks on Linux. Other changes and manual runs also
check formatting, Clippy, Rust tests on Linux and Windows, and Python tests.
The path classification is defined in
[`is_documentation`](../../scripts/ci/ci_scope.py); unavailable change history
selects full validation. The `check` job requires the selected checks to pass.
Further updates cancel an older run for the same PR or branch. Use the manual
trigger to check another branch before opening a PR. The
[PR title workflow](../../.github/workflows/pr-title.yml) checks pull request
titles against the [commit format](../../CONTRIBUTING.md#write-commits).

Run [Evaluation](../../.github/workflows/evaluation.yml) manually from Actions
for milestone acceptance, release validation, or substantial rule and parser
changes. Leave `baseline_ref` empty to compare diagnostics with the recorded
corpus run and benchmark only the selected revision. Supply a Git ref to run
diagnostic and performance comparisons against another revision; a pre-M2
revision uses its recorded M1 diagnostic report.

Evaluation saves complete reports as artifacts and adds diagnostic differences
to the workflow summary. Absolute latency targets remain acceptance checks;
relative performance changes are reported for review.

Enable `preview_stress` only when the dense duplicate workload needs another
measurement. It is separate from stable-rule latency acceptance; known
timeouts remain in the recorded evidence without delaying every evaluation.
For a local run, add `--preview-stress` to the benchmark command.
