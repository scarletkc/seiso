---
kind: adr
---

# Design history

The original design was written by @ScarletKc on September 27, 2026. Its
motivation was to make recurring documentation failures mechanically reviewable:
changing values copied into long-lived pages, repeated definitions, rationale
interrupting procedures, and traces of the writing process left in finished work.
The [ux-writing skill](https://github.com/scarletkc/agents/tree/main/skills/ux-writing)
informed those conventions; seiso ships its own rules and explanations and does
not depend on a skill or prompt.

The [original proposal](https://github.com/scarletkc/seiso/blob/9978f7db09d9077d861972e74948be9d43f6a3c9/docs/seiso%20设计与实施方案.md)
preserves the original Chinese rationale, candidate rules, milestones, and
tradeoffs. That dated proposal describes intentions, not the current interface.

The [documentation index](../README.md) organizes current guides, references,
evaluation records, and proposed work.

## Quality assurance

The [evaluation policy](../evaluation/policy.md) owns stable-promotion criteria,
regression requirements, and performance acceptance. The
[M1 protocol decision](../evaluation/m1-gate-proposal.md) retains its accepted
exception and evidence requirements. The
[milestone index](roadmap.md#milestone-evidence) links to dated results.
