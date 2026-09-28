---
kind: howto
---

# Verify the pinned corpus

The [corpus directory](../../corpus/README.md) contains the pinned source
manifest and fetch tool. Use Python 3.12 or later and a Rust toolchain
matching this checkout's requirements. Run from the repository root:

```sh
python corpus/corpus.py fetch
python corpus/corpus.py verify
python -m scripts.evaluation.run_corpus --corpus corpus --report target/corpus-forward.json
python -m scripts.evaluation.run_corpus --corpus corpus --report target/corpus-reverse.json --reverse
python -c "from pathlib import Path; assert Path('target/corpus-forward.json').read_bytes() == Path('target/corpus-reverse.json').read_bytes()"
```

The runner builds the current parser from the locked Cargo dependencies.
Every locked file is checked against its original bytes and hashes, then
parsed in a separate process. Both CommonMark and GFM are exercised, with
two parses per flavor. The probe validates source ranges, fragment mappings,
section/block references, and serialized-model equality.

The command exits with 0 only when every listed file passes. Missing or
changed inputs, panics, process failures, timeouts, model inconsistencies,
and nondeterministic output are failures. Individual failures are collected
while the remaining files are processed. `--jobs` controls concurrent
processes; `--timeout` bounds each process in seconds.

The JSON report identifies the corpus lock, parser source files, probe
binary, and per-document results by SHA-256. It contains no elapsed times or
timestamps, so reversed discovery order and process scheduling can be
checked by comparing complete reports from the same build.

The corpus directory owns [selection scopes, repository splits, and upstream
license references](../../corpus/docs/selection.md), alongside acceptance
artifacts. Source files are fetched by full commit and stored as verified
content blobs in the ignored `corpus/data/` directory. The parser does not
load or execute source-repository configuration or code.

See the [M0 acceptance record](../evaluation/m0-2026-09-27.md) for the initial
run. Parser acceptance does not measure rule precision or performance.

For rule precision and promotion, follow the
[single-file evaluation procedure](../../corpus/docs/evaluation.md).

## Exercise the rule engine

After fetching the corpus, run the optional rule-engine robustness test:

```sh
cargo test --test corpus --locked -- --ignored --nocapture
```

The test verifies each locked document's byte size and SHA-256, then checks
the unchanged source twice under each of two policies: its declared kind,
and a synthetic `howto` path mapping. The synthetic mapping exercises
convention rules; it does not assign human genre labels. Frontmatter still
takes precedence in both policies. Diagnostics and suppression records must
be byte-identical between checks, with valid source spans and coordinates.

`LNK001` is excluded because the blob cache does not contain complete
upstream file trees. Its filesystem behavior is covered by the
[engine fixtures](../../tests/engine.rs). This test reports
execution and diagnostic counts, not precision, and leaves the M0 acceptance
artifacts unchanged. Missing or modified corpus files fail the test.
