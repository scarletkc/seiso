---
kind: readme
---

# Documentation

Start with [checking documents](guides/checking.md) to configure seiso and read
its output. The [specification](../spec/convention.md) defines the document
responsibilities those checks enforce, and
[how seiso checks the convention](reference/convention.md) explains the
evidence behind each diagnostic.

## Specification

The [Seiso Convention Specification](../spec/convention.md) states the
convention independently of seiso, so that a project can follow it without
the tool and another checker can claim to check it. It is a provisional draft.
[Worked examples](../spec/examples.md) read its requirements against a small
documentation set, and the [adoption guide](../spec/adopting.md) applies them
to an existing repository. Its [changelog](../spec/CHANGELOG.md) records
changes to the specification separately from seiso releases.

## Guides

- [Check Markdown](guides/checking.md): select inputs and rules, inspect policy,
  consume diagnostics, and apply safe fixes.
- [Integrate checks](guides/integrations.md): configure agent hooks, pre-commit,
  and CI.
- [Develop seiso](guides/development.md): build, test, inspect parser behavior,
  and run performance or ecosystem comparisons.
- [Verify the pinned corpus](guides/corpus.md): reproduce parser evaluation.
- [Publish seiso](guides/publishing.md): prepare versions, validate distributions,
  publish, and recover a partial release.

## Reference

- [How seiso checks the convention](reference/convention.md): kind assignment,
  ownership precedence, evidence, rule availability, and requirement coverage.
- [Configuration](reference/configuration.md): discovery, inheritance, path
  mappings, comparison domains, and rule selection.
- [Architecture and execution](reference/architecture.md): modules, command
  input scope, links, suppression, fixes, and caching.

Rule explanations are stored by rule code in [rules/](rules/). Run
`seiso rule --all` to list implemented rules or `seiso rule CODE` to read one
rule's explanation and examples.

## Evaluation

The [evaluation policy](evaluation/policy.md) defines promotion and acceptance
criteria. The [rule evaluation procedure](../corpus/docs/evaluation.md) covers
fresh evidence and historical replay. Dated records preserve their results:

- [M0 parser acceptance](evaluation/m0-2026-09-27.md)
- [M1 rule acceptance](evaluation/m1-2026-09-28.md)
- [M1 protocol decision](evaluation/m1-gate-proposal.md)
- [M2 evaluation](evaluation/m2-2026-09-28.md)
- [M2 baseline decision](evaluation/m2-baseline-decision.md)
- [M3 heuristic evaluation](evaluation/m3-2026-09-28.md)
- [M3 context and abstention evaluation](evaluation/m3-optimization-2026-09-28.md)
- [Documentation site routes](evaluation/sites-2026-09-28.md)

## Design

The [design history](design/history.md) records the original motivation and
links to the original proposal. The [roadmap](design/roadmap.md) links milestone results and describes planned
integrations, classification experiments, and their acceptance criteria.
