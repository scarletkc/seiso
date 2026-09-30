---
kind: reference
---

# Evaluation policy

Rule promotion, implementation regression, performance acceptance, and registry
publication answer different questions. A passing test suite establishes
tested behavior; a frozen natural evaluation establishes measured precision;
a successful upload establishes distribution availability.
The [evaluation procedure](../../corpus/docs/evaluation.md) defines how to collect,
review, and replay evidence. Dated decisions and results remain in
[`docs/evaluation/`](./) as individual records.

## Stable promotion

New rules enter preview. Each rule requires its own evidence; results cannot be
pooled across rules. Several rules can share a holdout cohort, but each rule's
diagnoses are sampled, labeled, and scored separately. Natural promotion
requires at least 100 individually reviewed holdout diagnostics and precision
of at least 95%. Uncertain judgments cannot improve the score. One reviewer
labels every diagnosis. A second reviewer independently labels a random fifth
of them and every label other than a true positive, and the record reports
their agreement. Report true positives, false positives, uncertainty, sample
count, and precision separately for tuning and holdout, broken down by language
and document kind. With no samples, precision is unavailable.

Only tuning data may guide threshold changes. Changing behavior in response to
holdout findings requires fresh independent evaluation before claiming holdout
validation. A rule enters a holdout only after it meets the precision
requirement on tuning data. Corpus repositories are version-pinned with
original hashes and licenses; source diversity and annotation quality need
review alongside scores. Missed diagnoses are reported without a recall gate:
conservative reporting is preferred to unsupported diagnoses.

The [M1 protocol decision](m1-gate-proposal.md) grants a distinct
conformance route only to the named seiso declaration rules. Its positive,
negative, malformed-input, scope, disabled, incomplete, deterministic-output,
source-position, and natural-regression requirements remain binding.
Constructed cases never count toward natural sample sizes or precision.
Replaying that evidence against its accepted Git revision does not validate
the current implementation.

Heuristic promotion also requires usage-noise evidence: replay real document
changes and report incorrect blocks per 100 changes. Diagnostic comprehensibility
is assessed by giving an agent concise output without a writing skill. A repair
must pass checking and preserve unaffected identifiers, links, and numbers;
sampled human review must confirm that meaning survives. These evaluations are
promotion requirements, not claims that every proposed evaluation is automated.

## Regression coverage

Rule examples and complete diagnostic snapshots exercise both triggering and
non-triggering behavior. Cross-file fixtures use small workspaces. Changes to
snapshots are reviewed as behavior changes. Required regression cases include:

- Cache equality and order independence; moving unchanged content recomputes
  kind, domain, and relative destinations.
- Deleted targets and renamed headings, including incoming diagnoses when only
  the target file is selected; scoped results agree with full results.
- Required-input failures preserve incomplete suppression states, while local
  checks are unaffected by unrelated unreadable inputs.
- Disabled preview rules cannot make suppressions stale; safe edits preserve
  other codes and verify source content before writing.
- Canonical ownership and language/domain separation; the last matching kind
  mapping wins, and frontmatter cannot grant a generated exemption.
- Original byte ranges and Unicode columns across English, Chinese, Japanese,
  escapes, entities, and mixed Markdown fragments.

Any reproduced panic is a release blocker. Parser/model fuzzing is a planned
way to extend coverage; it is not an existing CI job. The exact routine checks
are defined in the [CI workflow](../../.github/workflows/ci.yml), with local commands
in [development](../guides/development.md#build-and-validate).

## Performance acceptance

The canonical workload size and absolute latency targets are `FILE_COUNT`,
`BYTES_PER_FILE`, and `TARGETS_SECONDS` in
[`scripts/evaluation/benchmark_m2.py`](../../scripts/evaluation/benchmark_m2.py). Measurements use the
default rule set, file-backed concise output, process startup for hook checks,
and the standard Linux CI runner. Record CPU model, core count, cold/warm cache
state, revision, and workload hashes with the result.

Preview rules are measured separately from stable latency acceptance. Dense
duplicate workloads require candidate filtering before expensive comparisons;
their timeouts remain visible evidence. Relative changes against a selected
baseline are reviewed separately from absolute acceptance. Scope differences
between historical engines are recorded in the
[baseline decision](m2-baseline-decision.md).

Run ecosystem comparisons for milestone acceptance, release validation, and
substantial parser or rule changes. Review added, removed, and changed diagnoses;
preserve complete reports alongside summaries. The
[development procedure](../guides/development.md#performance-and-ecosystem-checks)
provides the commands and manual workflow entry point.
