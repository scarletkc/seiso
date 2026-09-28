---
kind: adr
---

# M3 context and abstention evaluation

Recorded on 2026-09-28. Two optimization iterations reduce known false positives
by requiring stronger assertion and procedure evidence. All rules remain
preview. The final fresh cohort has no emitted diagnostics, so it does not
establish natural precision. Fresh section classification remains weak.

The [bound summary](../../corpus/results/m3/optimization-v2/summary.json)
separates development regressions from fresh evaluation. The
[original M3 record](m3-2026-09-28.md) and its evidence remain unchanged.

## Direction and implementation

The initial failures came from treating a version mention as a state snapshot,
a code language as proof of a reader procedure, and a recommendation as an
unsupported quality claim. Changing preamble length alone would not address
those errors.

The implementation now distinguishes:

- Version labels and explicit state relations from feature-introduction
  records and syntax examples. Deployment assertions need completion, current
  state, live state, or production context.
- Actual prose assertions from quoted examples, enumerated terms, and explicit
  sample output. Recommendations with no comparative claim or recognizable
  support remain undetermined.
- Reader instructions from console output and behavioral enumerations. A
  reference catalog does not need the same opening procedure as a how-to.
- A recognized procedure from an unclassified example that could already
  establish the main flow. Opaque containers and such examples leave ordering
  incomplete, preserving associated suppressions.

The classifier uses specific headings, contracts, tables, instructions, and
explanatory prose. Incidental words such as “alternative” or “return values”
do not independently establish a section responsibility. Definitions and
limits are in the [section reference](../reference/sections.md) and embedded
rule documentation. Numeric preamble and rationale thresholds are unchanged.

## Development regressions

The original corpus, including its former holdout, is development data for this
work. The [current run](../../corpus/results/m3/optimization-v2/known-corpus/run.json)
checks the same 1,953 documents and kind profiles. All 4,480 diagnoses from
earlier rules retain their complete diagnostic objects.

Of the original eleven M3 diagnoses, all ten false positives disappear and
the one uncertain EVD001 comparison remains. The original 38-section audit
improves from 18 to 34 matching labels. These are improvements on known cases,
not independent validation or population accuracy.

The [history replay](../../corpus/results/m3/optimization-v2/history/run.json)
uses the same 155 changes and fixed kind profile. Post-change false-positive
diagnoses fall from 26 to 6. Distinct incorrectly blocked changes fall from
7 to 6; several old warnings occurred together on the same edits. The remaining
VOX002 warnings concern the early design proposal's non-goals heading, under
the unchanged reference fallback. That rule still incorrectly blocks 6/155
changes, or 3.87 per 100. The other eight rules have no incorrect blocks in
this known replay.

## Fresh cohorts and freeze boundaries

The first iteration was fixed before reading the source content of HTTPX,
MkDocs, and Hatch. Their pinned, licensed cohort contains 127 documents.
Responsibility review used each document's opening and heading outline plus
complete representative documents, before diagnostic generation. Assignments
are agent judgments, not upstream declarations or human labels.

The [first fresh run](../../corpus/results/m3/optimization-v1/fresh-holdout/run.json)
produced twenty diagnoses, all judged false positives in
[individual review](../../corpus/results/m3/optimization-v1/fresh-holdout/labels.json).
This failed evaluation exposed historical version markers, naming conventions,
deployment-destination contracts, and early non-shell examples that the first
iteration still mishandled. Its complete results are preserved.

That cohort became development data for iteration two. With identical source
bytes and kind assignments, its
[development replay](../../corpus/results/m3/optimization-v2/first-cohort-replay/run.json)
has no diagnoses. This result cannot establish independent precision.

The [second freeze](../../corpus/results/m3/optimization-v2/freeze-final.json)
precedes fetching or reading the pipx/PDM cohort. Its 25 documents were selected
by the declared paths before content review. Previously reviewed repositories
are rejected by the cohort-pinning tool. The
[final fresh run](../../corpus/results/m3/optimization-v2/fresh-complete/run.json)
contains no diagnoses for any of the nine rules. Every rule therefore has
unavailable natural precision, not a perfect score. The cohort is small and
English-focused; rule recall has not been measured.

Completeness is retained separately: RAT001 is incomplete on nine files,
ORD001 and ORD002 on five each, and EVD001 on one. These counts overlap.
The completeness probe reproduces the initial frozen diagnostics and section
predictions exactly while exposing those execution states. Empty diagnostic
output does not mean every heuristic established an outcome.

## Fresh classification audit

The [fresh section audit](../../corpus/results/m3/optimization-v2/sections/summary.json)
contains eight stratified samples. Three predictions match the agent-reviewed
dominant responsibility. The other five confuse configuration reference with
background, miss prose instructions and site-tabbed examples, or interpret a
feature-list item as a procedure. One reference section remains unclassified.

This independent result is substantially weaker than the known-sample score.
It does not support broadening classifier-driven enforcement. No tuning
followed these final fresh judgments. Neither section sample is representative
enough to report population accuracy, and neither has human review.

## Verification and decision

Local checks pass 220 Rust tests and 141 Python tests, rustfmt, and strict
Clippy. Two opt-in corpus tests remain outside the routine Rust suite.
The nine published positive examples still trigger, and their repairs pass
all nine new rules together. Added cases cover contextual evidence, source
ranges, incomplete suppression states, version-history boundaries, original
CRLF input bytes, and rejection of previously reviewed repositories.

The repository's stable check passes. No standard Linux CI or new performance
acceptance run is claimed. No rule is promoted: natural positive samples,
independent repair comprehension, human meaning review, and reliable fresh
classification evidence remain absent.

The [verification receipt](../../corpus/results/m3/optimization-v2/verification.json)
binds the current implementation, tests, tools, and retained output. Reproduction
commands are in the [evaluation procedure](../../corpus/docs/evaluation.md#fresh-cohorts-for-heuristic-optimization).
