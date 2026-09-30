---
kind: reference
---

# How seiso checks the convention

seiso is the reference implementation of the
[Seiso Convention Specification](../../spec/convention.md). The specification
defines the document kinds, fact ownership, pointers, exceptions, and
conformance; a project can follow it without seiso. This page records the
choices seiso makes where the specification leaves them to the checker: how
configuration assigns kinds, how duplicate content is ranked, what evidence a
diagnostic claims, how exceptions are evaluated, which rules run by default,
and which requirements each rule covers. seiso does not identify AI
authorship, check spelling or formatting, or rewrite meaning. The convention
is independent of any writing skill or prompt; its original motivation is
recorded in the [design history](../design/history.md).

The specification in this repository is the version that a seiso built from
the same revision checks.

## Kind assignment

Frontmatter supplies a kind before a `[[kinds]]` path mapping, whose syntax
and precedence the [configuration reference](configuration.md#kinds-and-domains)
defines. `generated` can only be assigned in configuration. Generated documents
are exempt from rules but remain index sources, link targets, and possible
owners of duplicated facts. A document without a valid kind receives only the
kind-independent checks. See [KND001](../rules/KND001.md) and
[KND002](../rules/KND002.md) for executable examples.

## Ownership precedence

Duplicate and ownership checks compare documents within the same configured
domain and language. `lang` frontmatter takes precedence over script-based
language detection; Chinese and Japanese versions do not compete with English
versions merely because they name the same identifiers.

Ownership is evaluated per pair of duplicate blocks, without transitive
merging. The precedence is `canonical: true`, then generated, reference, ADR,
how-to, README, and other kinds. `canonical` applies to the whole file. Ties
remain unresolved and are reported by [OWN002](../rules/OWN002.md); Git age or
discovery order never decides ownership. The specification makes `canonical`
and the responsibilities of each kind normative and leaves the rest of this
ranking to the checker. Rule-specific matching belongs to
[DUP001](../rules/DUP001.md), [DUP002](../rules/DUP002.md),
[DUP003](../rules/DUP003.md), [OWN001](../rules/OWN001.md), and OWN002.

## Evidence

The strength of a diagnostic must not exceed its evidence. `seiso rule CODE`
shows each rule's basis:

| Basis | What the diagnostic claims |
| --- | --- |
| Consistency | A mechanically established fact: a missing kind, a missing link target, a malformed exception |
| Convention | A breach of a kind's content contract, without claiming that a value has actually expired |
| Heuristic | An observed feature and a conditional suggestion |

Semantic repairs remain the author's or agent's decision. Where a rule needs a
judgment, such as which of two tied documents owns a fact, its diagnostic
names the decision instead of making it.

## Exceptions

Exceptions name complete rule codes and give a reviewable reason. The syntax
and scope are defined by [SUP001](../rules/SUP001.md); completion and
unused-code semantics are defined by [SUP002](../rules/SUP002.md). Rule codes
are the accepted codes; specification identifiers such as `POINTER-1` are
not. A suppression cannot make an incomplete check complete: an incomplete
check keeps its [exit code](../guides/checking.md#consume-results), and SUP002
leaves its exceptions in the `incomplete` state.

## Rule availability

Rules are enabled or disabled; there is no warning tier. Stable rules are
selected by default. Preview rules require explicit preview opt-in as well as
selection. The [evaluation policy](../evaluation/policy.md) governs promotion.
`seiso rule --all` lists implemented rules with their status and embedded
explanations; the registry in [`src/rules/mod.rs`](../../src/rules/mod.rs)
owns rule availability, status, kind applicability, and input requirements.
Proposed rules belong to the [roadmap](../design/roadmap.md), and their codes
are not accepted configuration selectors.

## Requirement coverage

The table maps each requirement of the specification to the rules that check
it and states how much of the requirement those rules establish. Complete
means the rule decides the requirement from the documents alone. Partial
means the rule recognizes some breaches, usually through phrase lexicons or
definition-key matching, and the remainder is reviewed. Heuristic means the
rule observes a feature and suggests; the finding is a prompt for judgment.
A default check runs only the stable rules, so `seiso rule --all` shows which
rows a default check covers.

| Requirement | Rules | Coverage |
| --- | --- | --- |
| `KIND-1` one kind per document | KND001 | Complete |
| `KIND-2` declaration over mapping | KND001 | Complete; the mapping syntax is seiso's `[[kinds]]` |
| `KIND-3` broken declaration has no kind | KND001 | Complete |
| `KIND-4` `generated` only by mapping | KND002 | Complete |
| `KIND-5` content contract | RAT001, MIX001, VOX002 | Heuristic signals for some breaches; the contract is reviewed |
| `KIND-6` reader-directed, self-standing | VOX001, VOX003 | Partial: VOX001 matches requester phrases; VOX003 is heuristic |
| `KIND-7` no design argument in a procedure or contract | RAT002, RAT001, MIX001 | Partial: RAT002 matches design-choice headings; the others are heuristic |
| `KIND-8` procedure before background and recovery | ORD001, ORD002 | Heuristic |
| `FACT-1` one home per fact | DUP001, DUP002, DUP003 | Partial: definition blocks, restatement before a link, and near-duplicate paragraphs above thresholds; paraphrase is reviewed |
| `FACT-2` identifiable owner | OWN002 | Partial: ties are reported; the ranking above is seiso's |
| `FACT-3` comparison scope | `[[domains]]`, `lang` | Complete, as configured |
| `FACT-4` plans do not own shipped definitions | OWN001 | Partial: shared definition keys |
| `FACT-5` dated records hold history | none | Reviewed; `adr` and `changelog` are outside the STL rules |
| `FACT-6` evidence beside a claim | EVD001 | Heuristic: presence of nearby evidence, not its quality |
| `POINTER-1` no volatile values in long-lived documents | STL001, STL003, STL002, STL004 | Partial: STL001 and STL003 match marker lexicons; STL002 and STL004 are heuristic |
| `POINTER-2` pointers identify their target | PTR001, PTR003, PTR002 | Partial: repository roots and unnamed sources; PTR002 checks directories against `catalog-dirs` |
| `POINTER-3` links resolve | LNK001, LNK002 | Complete for local files and anchors; external URLs are not fetched |
| `EXCEPTION-1` complete codes, reason, scope | SUP001 | Complete |
| `EXCEPTION-2` an exception is a judgment | SUP001 | Partial: a missing reason or an invalid code; the reason's adequacy is reviewed |
| `EXCEPTION-3` remove unused exceptions | SUP002 | Complete for enabled rules that completed; `--fix` removes them |

For the requirements the specification places on checkers, this page and
`seiso rule --all` state the version and coverage (`CONFORMANCE-2`), the basis
table above distinguishes evidence (`CONFORMANCE-3`), an incomplete check
exits with code 2 and keeps exceptions incomplete (`CONFORMANCE-4`), and tie
diagnostics name the owner decision (`CONFORMANCE-5`). Thresholds, lexicons,
preview status, configuration, and output formats are seiso's
(`CONFORMANCE-6`).
