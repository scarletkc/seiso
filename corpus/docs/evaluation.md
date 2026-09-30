---
kind: howto
---

# Evaluate rules

Run from the seiso repository root with the Rust toolchain and Python 3.12
or later. Restore the pinned document bytes and complete file inventories:

```sh
python corpus/corpus.py fetch
python corpus/inventory.py fetch
```

Review document responsibilities before inspecting diagnostic output.
[Kind profiles](kinds.md) record the scope and provenance of those decisions.
Do not assign a genre merely to enable a rule.

## Freeze a run

```sh
python -m scripts.evaluation.evaluate_m1 --output corpus/reports/m1-candidate
```

The evaluator refuses to overwrite a run. It verifies the inventories,
checks each original document hash, builds the locked rule engine, and runs
the same inputs twice. The saved report binds diagnostics to source,
configuration, corpus, executable, and inventory hashes. Link existence
uses case-sensitive Git paths; symlinks and submodule contents are
undetermined. Upstream programs and configuration files are not executed.

## Review diagnostics

Annotate each diagnosis as `tp`, `fp`, or `uncertain`, with a reason and its
diagnostic ID. Store annotation files as `labels-*.json` beside the frozen
report. Bind them to the compressed report's SHA-256 in `report_sha256`.
Record the review method and the IDs inspected individually. Agent review
and independent consistency oracles are identified separately from human
labels.

Use only tuning findings to develop rule behavior. A holdout finding can
prevent promotion; changing a rule in response requires a fresh holdout
evaluation before claiming independent validation.

## Summarize the gates

```sh
python -m scripts.evaluation.summarize_m1 corpus/reports/m1-candidate
```

The summarizer rejects missing, duplicate, and unbound annotations. It
reports tuning and holdout separately, including language/kind groups and
unresolved judgments. Unknown labels cannot improve the promotion score.
Natural precision is unavailable when there are no samples.

An accepted protocol exception is supplied explicitly with `--protocol`.
The acceptance receipt must match the syntax audit and conformance test
sources at its accepted Git revision. New receipts record that commit in
`source_revision`; receipts without it require `--protocol-source-ref`.
The summary records the resolved commit and receipt hash. Constructed
protocol cases never enter natural precision counts.
See the [M1 protocol decision](../../docs/evaluation/m1-gate-proposal.md).

### Replay historical evidence

Replay reads accepted test sources from Git, so subsequent moves or refactors
do not invalidate a frozen receipt. It verifies historical evidence and does
not establish that the current implementation passes those tests. Keep the
original report, annotations, acceptance receipt, and summary unchanged; use
`--output` for a new replay summary.

The checked-in M1 receipt predates `source_revision`. Its accepted test hashes
match commit `b5651d290a261d603479d4b75538277a49dd0725`:

```sh
python -m scripts.evaluation.summarize_m1 corpus/results/m1/natural-v1 --protocol corpus/results/m1/protocol-acceptance.json --protocol-source-ref b5651d290a261d603479d4b75538277a49dd0725 --output target/m1-replay.json
```

Fetch that revision if a shallow checkout does not contain it. To check current
protocol behavior, run the current contract tests separately:

```sh
cargo test --locked --test engine --test suppression --test acceptance_contract
```

Fresh rule-promotion claims also require a new evaluation and review against the
current implementation. A historical summary is not a replacement for them.

## Cross-file evaluation

```sh
python -m scripts.evaluation.evaluate_m2 --output corpus/reports/m2-candidate
```

Each upstream repository forms its own workspace. The index uses the pinned
Git inventory for physical paths and the original selected Markdown for
anchors and duplicate content. An existing file outside the document sample
has unknown anchors. Symlinks and submodules remain undetermined.

The evaluator reverses source, document, and inventory order for its second
run and requires identical output. It applies single-file and cross-file
diagnostics together before suppression. It records all implemented preview
rules; new rules remain preview until their independent acceptance gates pass.

Review every new rule's diagnoses with the original primary and related
source ranges. Report TP, FP, uncertain, sample count, precision, and
language/kind groups for each split. A rule with no holdout diagnoses has
unavailable precision, rather than a perfect score.

Stage completion and stable promotion are separate decisions. A complete
report may establish that natural samples are absent or that a rule is too
noisy. Keep that rule in preview; constructed cases do not fill the natural
sample requirement. Promotion also needs a review of source diversity and
annotation quality, not just a numeric score.

Compare two reports over the same locked inputs:

```sh
python -m scripts.evaluation.ecosystem_diff before/diagnostics.json.gz after/diagnostics.json.gz --output corpus/reports/diff
```

The JSON retains every added, removed, and changed diagnosis, including
related locations. The Markdown view limits individual entries for workflow
summaries; the artifact contains the full result. Changing corpus locks or
kind profiles requires a separate review before comparing rule behavior.

## Documentation site routes

[`evaluation/sites.json`](../evaluation/sites.json) gives a `[[sites]]` entry
to each source whose pinned tree has a site generator configuration or
navigation file covering its corpus documents. Each entry cites that file as
its evidence. Pass the profile to resolve links as site routes:

```sh
python -m scripts.evaluation.evaluate_m3 --split all --sites corpus/evaluation/sites.json --output corpus/reports/sites
```

The run records the profile's SHA-256. Without `--sites`, the evaluator's
inputs and report are unchanged, so `ecosystem_diff` between runs with and
without the profile isolates route resolution. Review the entries like kind
profiles before comparing rule behavior.

## Heuristic evaluation

Freeze tuning output before reviewing the holdout. The M3 evaluator includes
section predictions for every document, including documents without a kind:

```sh
python -m scripts.evaluation.evaluate_m3 --split tuning --output corpus/reports/m3-tuning
python -m scripts.evaluation.calibrate_m3 --output corpus/reports/m3-calibration
```

The calibration script compares existing duplicate-rule thresholds on tuning
repositories only. Inspect candidate diagnoses before changing defaults;
zero samples cannot establish a preferred threshold. Preserve each tuning
iteration and its labels.

After fixing the implementation, freeze the full run and collect real Git
document changes through an explicit commit:

```sh
python -m scripts.evaluation.evaluate_m3 --split all --output corpus/reports/m3-natural
python -m scripts.evaluation.replay_m3 --revision HEAD --output corpus/reports/m3-history
```

The replay stores original before/after source, commit identities, a fixed
kind profile, complete diagnoses, and introduced diagnoses. It ignores
position-only movement of unchanged warnings. The fixed profile applies to
both revisions; it does not reconstruct historical project configuration.
Only repository Markdown is read, and no historical code is executed.

Label all new heuristic diagnostics and every post-change replay diagnostic. M3
labels require `id`, `code`, `input_sha256`, `diagnostic_sha256`, `label`, and
`reason`; the bundle records `schema_version`, `report_sha256`, and
`reviewer_kind`. Hashes use SHA-256; diagnostic hashes use `encode` from
[`evaluate_m2.py`](../../scripts/evaluation/evaluate_m2.py). Then summarize:

```sh
python -m scripts.evaluation.summarize_m3 --report corpus/reports/m3-natural/diagnostics.json.gz --labels corpus/reports/m3-natural/labels.json --replay corpus/reports/m3-history/replay.json.gz --replay-labels corpus/reports/m3-history/labels.json --output corpus/reports/m3-summary.json
```

An incorrect block is one document change with a false positive after checking,
including a warning that persists from the previous revision. Several false positives on one change count once. The denominator
includes clean changes. Uncertain changes are also counted in a separate
conservative rate. These are simulated blocks with preview rules enabled,
not observed user interruptions.

Sample section predictions independently of whether a rule reported them:

```sh
python -m scripts.evaluation.sections_m3 sample --report corpus/reports/m3-natural/diagnostics.json.gz --output corpus/reports/m3-sections/sample.json
python -m scripts.evaluation.sections_m3 summarize --report corpus/reports/m3-natural/diagnostics.json.gz --sample corpus/reports/m3-sections/sample.json --labels corpus/reports/m3-sections/labels.json --output corpus/reports/m3-sections/summary.json
```

Between those commands, label every sample with `expected_type` and `reason`,
preserving its `id`, `input_sha256`, and `annotation_sha256`. The label bundle
records `sample_sha256` and `reviewer_kind`. The sampler includes unclassified
content and a type-independent sample so missed responsibilities stay visible.
Its stratified confusion table is not a population accuracy or rule-recall
estimate.

[`tests/repairs.rs`](../../tests/repairs.rs) checks authored repairs and protects
unaffected facts. The independent concise-output agent trial and sampled
human meaning review required for promotion remain separate evaluations.

Replay the frozen M3 outcomes against a changed implementation without
relabeling or replacing the original reports:

```sh
python -m scripts.evaluation.verify_m3 --natural corpus/reports/m3-natural/diagnostics.json.gz --history corpus/reports/m3-history/replay.json.gz --output target/m3-revalidation.json
```

This command compares natural diagnostics, section predictions, history results,
and introduced-warning matching, including reversed-input equality. Added
`incomplete_rules` metadata is compatible with reports that predate that field;
when both reports contain it, its values must agree. Review behavior differences
as regressions or intentional changes. Fresh holdout evidence is needed when
claiming independent validation after tuning, not for a report-format change.

## Fresh cohorts for heuristic optimization

Once a holdout finding guides an implementation change, treat that cohort as
development data. Preserve its original report and labels. Choose new public
repositories and fixed path scopes before reviewing their content:

```sh
python -m scripts.evaluation.fresh_m3 pin --source OWNER/REPO --output corpus/reports/new-cohort/selection.json
python -m scripts.evaluation.fresh_m3 fetch --input corpus/reports/new-cohort/selection.json --output corpus/reports/new-cohort/corpus.lock.json
```

Pinning rejects repositories from the original corpus and retained M3 cohort
locks. Supply `--previous-corpus PATH` for other prior cohorts. Freeze the
implementation before inspecting new source text. Review kinds before
producing diagnoses. A kind profile needs `corpus_sha256` and a `documents`
map keyed by `source-id/path.md`; each decision records `kind`, `reason`, and
`input_sha256`. Use `unknown` when the document responsibility is unclear.
The [retained profile](../results/m3/optimization-v2/fresh-inputs/kinds.json)
also records historical implementation and review-coverage metadata.
Kind judgments are independent of implementation identity. Each evaluation
records its own implementation fingerprints using LF-normalized source text.
Document and license hashes always cover their original bytes. Older reports
retain their original fingerprints; they are historical records, not a gate on
the current checkout.

```sh
python -m scripts.evaluation.fresh_m3 evaluate --input corpus/reports/new-cohort/corpus.lock.json --profile corpus/reports/new-cohort/kinds.json --output corpus/reports/new-cohort/run
```

The evaluator verifies original bytes, checks that the implementation stays
unchanged during the run, and checks reversed input order. It retains per-file
incomplete rule states. Use `--development-replay` when reusing an earlier
cohort, including with an unchanged kind profile; its diagnoses are tuning
data. No external repository code runs.

For section sampling on a separate cohort, pass `--corpus-lock PATH` to
`sections_m3.py sample` and `sections_m3.py summarize`. Original blobs remain
in the shared verified corpus cache. Label bundles retain the same source and
prediction bindings as the original section audit.

Summarize stored reports using an input manifest:

```sh
python -m scripts.evaluation.summarize_m3_optimization --manifest corpus/results/m3/optimization-v2/summary-inputs.json --output target/m3-optimization-summary.json
```

The manifest names `comparisons`, `cohorts`, `histories`, and `sections`, with
paths relative to the manifest. Each comparison and history names `before`
and `after` report/label pairs. Cohorts name a report, labels, and corpus lock;
section audits name a report, sample, and labels. Set `preserve_existing_rules`
on a comparison when unchanged non-M3 diagnostics are part of its contract.
The [retained manifest](../results/m3/optimization-v2/summary-inputs.json)
provides a complete example.

The summary validates the named inputs and label bindings without loading the
current engine or requiring its source checkout and corpus cache. Its input
list contains only files it consumes. Cohort roles come from their reports;
empty samples retain unavailable precision. Source and classification reviews
remain agent judgments unless their label bundles record human review.

## Fresh link cohorts

`scripts.evaluation.fresh_links` runs an independent LNK002 holdout. Freeze
the implementation and commit the selection specification before pinning a
source or reading its content. No upstream code runs.

1. Pin a batch and fetch its documents. Pinning rejects any repository that an
   earlier corpus or batch used.

   ```sh
   python -m scripts.evaluation.fresh_links pin --spec corpus/results/lnk002/fresh-v1/sources.json --batch 1 --output corpus/results/lnk002/fresh-v1/batch-1/selection.json
   python -m scripts.evaluation.fresh_links fetch --input corpus/results/lnk002/fresh-v1/batch-1/selection.json --output corpus/results/lnk002/fresh-v1/batch-1/corpus.lock.json
   ```

2. Write `sites.json` from each source's pinned generator configuration, and
   `kinds.json` with every document `unknown`. `validate_sites` and
   `kind_mappings` in `fresh_links.py` define their fields.
3. Evaluate the batch. Add the next batch only while the total LNK002 count is
   below 100.

   ```sh
   python -m scripts.evaluation.fresh_links evaluate --input corpus/results/lnk002/fresh-v1/batch-1/corpus.lock.json --profile corpus/results/lnk002/fresh-v1/batch-1/kinds.json --sites corpus/results/lnk002/fresh-v1/batch-1/sites.json --output corpus/results/lnk002/fresh-v1/batch-1/run
   ```

4. Label every LNK002 diagnosis on the page its readers see: the site's output
   when a site entry covers the target, and GitHub otherwise. `tp` means nothing
   on that page matches the fragment, `fp` means something does, and
   `uncertain` means the pinned sources cannot tell, with the missing evidence
   named. Two agents label independently; `validate_dual_review` lists the
   required fields. Cite third-party files by repository, commit, and path
   instead of copying them. For GitHub-only first evidence, run
   `corpus/evaluation/review_m2_links.py --candidate` with `--corpus-lock` and
   `--inventory` on the `files` list from the report.
5. Bind the labels to the report and summarize:

   ```sh
   python corpus/evaluation/review_m2_links.py --bind-report corpus/results/lnk002/fresh-v1/batch-1/run/diagnostics.json.gz --decisions corpus/results/lnk002/fresh-v1/batch-1/decisions.json --output corpus/results/lnk002/fresh-v1/batch-1/labels.json
   python -m scripts.evaluation.fresh_links summarize --manifest corpus/results/lnk002/fresh-v1/manifest.json --output corpus/results/lnk002/fresh-v1/summary.json
   ```

Record the results under `docs/evaluation/`. A failed gate keeps the rule in
preview, and a fix needs another fresh cohort. Before a record merges, rebase
on `main` and rerun every batch; the raw results must be byte-identical.

To rerun a recorded batch, restore its documents from the lock instead of
pinning it again, which the overlap check rejects. Evaluate into a new
directory, then compare `raw_result_sha256` in its `run.json` with the
recorded one:

```sh
python -c "import json; from corpus.corpus import fetch_blob; lock = json.load(open('corpus/results/lnk002/fresh-v1/batch-1/corpus.lock.json', encoding='utf-8')); [fetch_blob(s, e) for s in lock['sources'] for e in s['documents'] + s['licenses']]"
python -m scripts.evaluation.fresh_links evaluate --input corpus/results/lnk002/fresh-v1/batch-1/corpus.lock.json --profile corpus/results/lnk002/fresh-v1/batch-1/kinds.json --sites corpus/results/lnk002/fresh-v1/batch-1/sites.json --output target/lnk002-replay
```
