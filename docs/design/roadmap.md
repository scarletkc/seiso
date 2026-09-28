---
kind: plan
---

# Roadmap

This page contains proposed work. Implemented behavior is described by the
[convention](../reference/convention.md), [configuration reference](../reference/configuration.md),
[architecture](../reference/architecture.md), and `seiso rule --all`. The original phased
proposal is preserved through the [design history](history.md).
Milestone numbers describe evaluation stages and do not establish a published
version or require a particular Cargo package layout.

## Milestone evidence

The [M0 record](../evaluation/m0-2026-09-27.md),
[M1 record](../evaluation/m1-2026-09-28.md),
[M2 record](../evaluation/m2-2026-09-28.md), and
[M3 record](../evaluation/m3-2026-09-28.md) retain their measured outcomes,
limitations, and decisions. Stage completion and stable-rule promotion remain
separate. The [evaluation policy](../evaluation/policy.md) owns acceptance criteria.

## M3: Heuristic rules and section classification

The [M3 record](../evaluation/m3-2026-09-28.md) records the implemented
classifier, nine preview rules, threshold calibration, edit-history replay,
repair regressions, and per-rule precision and usage-noise reports.
The [optimization record](../evaluation/m3-optimization-2026-09-28.md) records
subsequent context handling, abstention, and fresh-cohort limitations.
[Section annotations](../reference/sections.md) describe the inspection API;
`seiso rule --all` owns the available rules and their explanations.

M3's reporting criterion is a holdout precision report and a usage-noise
report for every proposed heuristic. Stage completion does not promote rules
together: each rule must independently satisfy the
[evaluation policy](../evaluation/policy.md). Missing natural samples and
reported false positives remain limits on promotion. Subsequent rule tuning
needs fresh holdout evidence before claiming validation.

## M4: Ecosystem

M4 delivers editor and agent integrations and evaluates the optional
classification backend below. Each subproject is accepted independently.

### Editors and agent hooks

Planned integrations include `seiso server` (LSP), a thin VS Code extension,
and hook adapters for Codex CLI, Cursor, and other agents. Each integration
needs a defined input/output contract, passing diagnostic refresh tests,
and latency evidence before acceptance.

### Classification backend experiment

An external classifier may be evaluated after a section-labeled baseline
exists. [Jev](https://openrouter.ai/typesafe/jev-1.13) was a candidate in the
original proposal. The experiment would classify candidate sections, with
deterministic code deciding whether evidence meets a calibrated rule threshold.
Model confidence alone is not a validated threshold. It would not rewrite prose.

Evaluation would record backend/model identity and every input affecting the
answer: paragraph, heading, kind, context, and question. A frozen mode for CI
would consume only accepted recorded answers; absent answers would remain
undetermined. Compare precision, cost, and latency against the same heuristic
baseline before adopting a backend. A trait, new crate, configuration syntax,
or `--judge` option is chosen after the experiment establishes a useful result.
An experiment can conclude without adopting a model backend; retain its
measured result and decision as its acceptance record.

## Open design questions

- Whether projects need custom kinds such as tutorials or specifications.
- Whether versioned documentation trees need automatic comparison domains.
- Whether MDX support justifies an additional parser surface.
- Whether rule explanations should be distributed in Chinese and Japanese.
- Target dates and ownership for future deliverables.

The project's license is recorded in [LICENSE](../../LICENSE). Parser replacement
remains possible behind the document model if measured correctness, maintenance,
or performance needs justify it. Adoption should remain incremental through
path mappings and selectors. Policy review must keep exemptions, ignored paths,
and generated mappings visible so passing checks still have a meaningful scope.
