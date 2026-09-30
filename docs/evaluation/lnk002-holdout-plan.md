---
kind: plan
---

# LNK002 fresh holdout plan

Proposed on 2026-09-30 for [#11](https://github.com/scarletkc/seiso/issues/11)
and approved by the project owner. This file is a work order. Delete it in the
final commit of this branch: the evaluation record and the procedure section
described below replace it.

The owner amended the implementation baseline on 2026-09-30: complete the
reviewable work on this branch and keep every PR unmerged. After stage 2
review, integrate the verified heading-name fix from draft PR #56 into this
branch and use its exact commit as the freeze baseline. This replaces the
requirement to merge that fix into `main` before stage 3. It does not change
the source selection, freeze ordering, blind dual review, batch stopping
point, conservative precision, or diversity gates. `main` remains unchanged.

## Goal

Measure LNK002 on repositories selected after its implementation is frozen,
and promote it to stable if the
[stable promotion policy](policy.md#stable-promotion) is met: at least 100
individually reviewed holdout diagnoses and a conservative precision of at
least 95%. Conservative precision divides true positives by all diagnoses, so
an uncertain label counts against the rule. LNK002's basis is consistency,
not heuristic, so the usage-noise replay and the concise-output agent trial
do not apply.

## Decisions

- **Kinds.** LNK002 applies to every kind and to documents without one. Every
  cohort document gets kind `unknown`, with the reason "LNK002 applies to every
  kind; document responsibilities were not reviewed for this link-rule
  holdout." The record reports the kind breakdown as unavailable.
- **Sites.** Each source gets reviewed `[[sites]]` entries, using the method of
  [`corpus/evaluation/sites.json`](../../corpus/evaluation/sites.json): an
  entry needs a generator configuration or navigation file at the pinned commit
  as its evidence. Sources without one get no entry.
- **Renderer for labels.** A diagnosis is judged on the page that readers of
  the source actually open. When a reviewed site entry covers the target page,
  that is the site generator's output; otherwise it is GitHub's repository view
  at the pinned commit.
- **Selection.** The repositories and path scopes in [Selection](#selection)
  are fixed now, before anyone reads their Markdown or diagnostics.

## Stages

Each stage ends at a checkpoint where the reviewing agent checks the result
before the next stage starts. Stages 1 and 2 may run in parallel.

### 1. Decide heading `name` attributes

All 32 uncertain M2 tuning diagnoses in `ripgrep` link to fragments that exist
only as a `name` attribute on an `<h3>` element, such as `FAQ.md#config`.
LNK002 counts `name` only on `<a>` elements. Settle this before the freeze,
because an uncertain label fails the gate.

Collect and store this evidence under `corpus/results/lnk002/name-attribute/`:

1. The HTML that GitHub's Markdown API (`gh api markdown`, GFM mode, context
   `BurntSushi/ripgrep`) returns for the exact heading bytes in `FAQ.md` at the
   corpus's pinned commit, `3fce3b5bb0236da2df6d99672afb8a719642eca7`.
2. A headless-browser check of
   `https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/FAQ.md#config`:
   the scroll position and the target heading's position after load, compared
   with a fragment that exists as an ordinary heading slug and one that
   matches nothing.
3. The HTML standard's rule for the element a fragment indicates, which is what
   static site pages follow.

If GitHub scrolls to the heading, change LNK002 to accept a `name` attribute on
any element as an anchor, since seiso prefers a missed diagnosis to a false
one. Make that change in a separate `fix(rules)` pull request that also updates
the anchor list in [LNK002](../rules/LNK002.md#what-it-checks) and the oracle's
explicit-anchor extraction in `corpus/evaluation/review_m2_links.py`. Keep that
PR unmerged and integrate its verified change into this branch before stage 3,
following the owner's amended baseline instruction above. If GitHub does not scroll, change nothing; these links are
broken for readers.

The two uncertain `ruff` diagnoses need no separate decision. They depend on
how a site generator slugs trailing whitespace in a heading, which the renderer
decision above already covers.

### 2. Build the cohort tooling

Add `scripts/evaluation/fresh_links.py`. The existing
`scripts/evaluation/fresh_m3.py` only runs the single-file M3 rules, and
`scripts/evaluation/evaluate_m2.py` only reads the main corpus. The new
script has four subcommands:

- `pin --spec SPEC --batch N --output DIR/selection.json` resolves the batch's
  sources with `resolve_source` from `corpus/corpus.py` and stores each
  source's recursive Git tree from that same resolution, so the inventory
  matches the pinned commit. It refuses a truncated tree, a missing license
  file, a source with no matching Markdown, and any repository, compared
  case-insensitively, that appears in `corpus/corpus.lock.json`, in any
  `corpus/results/**/corpus.lock.json` or `selection.json`, or in an earlier
  batch.
- `fetch --input DIR/selection.json --output DIR/corpus.lock.json` stores
  original blobs in the shared `corpus/data/blobs` cache through `fetch_blob`,
  records each SHA-256, and writes inventory archives with an inventory lock in
  the format `corpus/inventory.py` uses.
- `evaluate --input DIR/corpus.lock.json --profile DIR/kinds.json --sites
  DIR/sites.json --output DIR/run` runs the `evaluate_m2` example probe. Each
  source is its own workspace with the configuration `preview = true`, its
  reviewed `[[sites]]` entries, and `[lint] select = ["LNK001", "LNK002"]`.
  Kind `unknown` adds no `[[kinds]]` entry. Like `evaluate_m2.py`, it must
  produce byte-identical output with sources, documents, and tree entries
  reversed, confirm that implementation fingerprints did not change during
  the run, refuse to overwrite a run, and bind the report to the lock,
  inventory, profile, site profile, script, and probe hashes. Rows keep the
  `evaluate_m2.py` diagnostic schema, including `links`, `related_inputs`, and
  `incomplete_rules`, with `split` set to `holdout`.
- `summarize --manifest MANIFEST --output summary.json` reads every batch's
  report and bound labels and reports TP, FP, uncertain, samples, precision,
  and conservative precision overall, per batch, and per source, plus the
  per-language breakdown. It states whether the gate in
  [Gate and promotion](#gate-and-promotion) passes. It rejects missing,
  duplicate, or unbound labels.

Reuse the helpers in `evaluate_m2.py` (`prepare_inputs`, `diagnostic_rows`,
`reverse_inputs`, `verify_blob`) and generalize them only where a separate
cohort needs it. Existing runs must stay reproducible: running `evaluate_m2`
on the main corpus must give the same raw result before and after the change.
Give `review_m2_links.py` optional corpus-lock and inventory arguments whose
defaults keep its current behavior.

Add `scripts/tests/test_fresh_links.py` without network access, mocking the
GitHub calls. Cover overlap rejection, specification validation, profile
binding (missing, extra, and changed documents), unknown kinds producing no
`[[kinds]]`, site rendering, refusal to overwrite, and detection of a
reverse-order difference.

Document the commands in a "Fresh link cohorts" section of
[the evaluation procedure](../../corpus/docs/evaluation.md). That section
replaces this stage's description once the plan is deleted.

### 3. Freeze and pin

The freeze commit is this branch after the verified stage 1 fix and reviewed
stage 2 tooling, based on the unchanged `main`. Record it in the run metadata,
and state that it is a branch baseline rather than an already-merged `main`
implementation. From then until the labels are
complete, this branch changes nothing under `src/`, `docs/rules/`, or
`examples/evaluate_m2.rs`.

Commit the selection specification, `corpus/results/lnk002/fresh-v1/sources.json`,
before pinning anything, so the history shows the selection came first. Then
pin and fetch batch 1.

### 4. Review site entries

For each pinned source, read only its generator configuration or navigation
files at the pinned commit (for example `mkdocs.yml`, `.vitepress/config.*`,
`docusaurus.config.*`, `book.toml`, or an Eleventy or Hugo configuration) and
write `sites.json` bound to the batch's lock. Write `kinds.json` in the
per-document format that `fresh_m3.py evaluate` accepts, with every document
`unknown`.

### 5. Evaluate until the sample is large enough

Evaluate batch 1 and look only at the LNK002 count in `run.json`. If the total
across evaluated batches is below 100, pin, review, and evaluate the next
batch. Stop at the first batch that brings the total to 100 or more. If all
batches are used first, the evaluation ends with an insufficient sample and
LNK002 stays in preview. Every diagnosis in every evaluated batch is labeled.

### 6. Label every diagnosis

Run the oracle on each batch, then review every LNK002 diagnosis individually
against the original source, the target, and the renderer from
[Decisions](#decisions):

- `tp`: the fragment matches no element on the rendered target page.
- `fp`: it matches, through that renderer's heading slugs, a declared custom
  id, an HTML `id`, an `<a name>`, or the stage 1 outcome.
- `uncertain`: resolution depends on behavior that the pinned sources cannot
  establish, such as a client-side component or content generated at build
  time. Name the missing evidence.

Labels follow the schema of `corpus/results/m2/natural-v1/labels-links.json`,
with two added fields, `renderer` and `evidence`, and are bound with
`review_m2_links.py --bind-report`. The implementing agent labels first. The
reviewing agent then labels every diagnosis independently; the bundle keeps
both labels, the resolved label, and the reason for each disagreement. The
summary uses resolved labels and reports the agreement rate. Record
`human_reviewers: 0` unless the owner labels.

### 7. Summarize, record, and decide

Write the summary and a dated record, `docs/evaluation/lnk002-<date>.md`, in
the style of the [sites record](sites-2026-09-28.md). The record gives the
inputs, the stage 1 decision and its evidence, results per batch and source,
the causes of every false positive and uncertain label, and the decision.

## Gate and promotion

The gate passes when all of these hold:

- At least 100 LNK002 holdout diagnoses, each labeled by both agents.
- Conservative precision of at least 95%.
- A source-diversity review in the record. If one source contributes more than
  half of the diagnoses, the owner decides whether the sample is diverse
  enough.

If it passes, a separate commit sets `stable: true` for LNK002 in
`src/rules/mod.rs`. It also replaces the preview instruction in
[LNK002's configuration section](../rules/LNK002.md#configuration) with
wording like LNK001's, and updates any other text that lists which rules are
preview or stable. Rerun every batch after the change and write
`corpus/results/lnk002/fresh-v1/promotion.json` in the shape of
`corpus/results/m1/promotion.json`, confirming that diagnostics and file
results are unchanged. Leave versions and release notes to the release.

If it fails, change no rule code. The record lists the false-positive causes
as tuning findings. Any fix needs another fresh cohort before a promotion
claim.

## Selection

Every source includes its root `README.md` and the path scope below. A source
that fails a mechanical check in `pin` is skipped and recorded, not replaced.

| Batch | Repository | Path scope |
| ---: | --- | --- |
| 1 | `nodejs/node` | `doc/**` |
| 1 | `electron/electron` | `docs/**` |
| 1 | `microsoft/vscode-docs` | `docs/**` |
| 1 | `fastapi/fastapi` | `docs/en/docs/**` |
| 1 | `squidfunk/mkdocs-material` | `docs/**` |
| 1 | `traefik/traefik` | `docs/content/**` |
| 1 | `rust-lang/cargo` | `doc/book/src/**` |
| 1 | `rust-lang/rustc-dev-guide` | `src/**` |
| 1 | `vitest-dev/vitest` | `docs/**` |
| 1 | `eslint/eslint` | `docs/src/**` |
| 2 | `argoproj/argo-cd` | `docs/**` |
| 2 | `backstage/backstage` | `docs/**` |
| 2 | `open-telemetry/opentelemetry.io` | `content/en/docs/**` |
| 2 | `external-secrets/external-secrets` | `docs/**` |
| 2 | `rook/rook` | `Documentation/**` |
| 3 | `rust-lang/reference` | `src/**` |
| 3 | `microsoft/TypeScript-Website` | `packages/documentation/copy/en/**` |
| 3 | `microsoft/playwright` | `docs/src/**` |
| 3 | `pnpm/pnpm.io` | `docs/**` |
| 3 | `pydantic/pydantic` | `docs/**` |
| 4 | `vitejs/vite` | `docs/**` |
| 4 | `jestjs/jest` | `docs/**` |
| 4 | `rust-lang/rustup` | `doc/user-guide/src/**` |
| 4 | `grafana/grafana` | `docs/sources/**` |
| 4 | `home-assistant/home-assistant.io` | `source/**` |

None of these repositories is in an earlier corpus or cohort, and none
translates or derives from one.

## Files

```text
corpus/results/lnk002/
  name-attribute/          stage 1 evidence
  fresh-v1/
    sources.json           selection specification, committed before pinning
    batch-N/
      selection.json       pinned sources and trees
      corpus.lock.json     fetched lock with SHA-256
      inventory/           inventory lock and tree archives
      kinds.json
      sites.json
      run/                 diagnostics.json.gz and run.json
      oracle.json
      labels.json
    summary.json
    promotion.json         only if the gate passes
docs/evaluation/lnk002-<date>.md
```

## Constraints

- Nobody reads cohort Markdown or diagnostic text before the freeze commit and
  the selection specification are committed. Before evaluation, the only
  cohort files read are the generator configuration files in stage 4.
- No upstream code or configuration runs; configuration files are only read.
- Python 3.12 or later with the standard library; the oracle keeps its pinned
  `markdown-it-py` and `github-slugger` versions.
- Before the branch merges, rebase it on `main` and rerun `evaluate` for every
  batch. The raw results must be byte-identical to the labeled runs. If they
  differ, stop and report the difference instead of relabeling.
- Commits follow Conventional Commits, one stage per commit or more.
