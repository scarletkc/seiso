---
kind: reference
---

# Seiso Convention Specification

This specification restates the convention that [seiso](../README.md) checks,
so that a project can follow the convention without installing seiso or any
other tool, and so that a checker other than seiso can claim to check it. It is
provisional: before 1.0.0, a release can change its wording, requirement
identifiers, and scope, as [Versioning](#versioning) describes. The
[changelog](CHANGELOG.md) lists the released versions and the changes not yet
released. The [roadmap](../docs/design/roadmap.md#convention-specification)
records the open questions and the criteria for continuing, narrowing, or
closing this exploration.

The convention applies to the Markdown documents of a software project: the
README, guides, references, runbooks, agent instructions, decision records,
plans, and changelogs that people and coding agents read as context for their
next change. It specifies how those documents divide responsibilities, where a
fact lives, how a document refers to information that changes without it, how
an exception to a requirement is recorded, and what a claim of conformance
means. It does not specify spelling, formatting, prose style, Markdown syntax,
or how a checker parses, configures, or reports.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are to be
interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119)
and [RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
capitals. Each requirement carries an identifier such as `KIND-1`. Identifiers
are stable within a version of this specification, so that a checker can state
which requirements it covers and a reader can cite one. Paragraphs without an
identifier explain; they add no requirements.

## Terms

- **Documentation set**: the documents a conformance claim covers. Ordinarily
  the Markdown files of one repository, less excluded paths such as vendored
  or test material.
- **Kind**: the responsibility a document declares, taken from
  [Document kinds](#document-kinds).
- **Long-lived document**: a document maintained so that it stays true for as
  long as it exists.
- **Dated record**: a document that preserves what was decided or observed at
  a point in time.
- **Proposal**: a document that describes intended work.
- **Fact**: a statement a reader may act on: a definition, an accepted value, a
  default, a requirement, a step, a decision.
- **Authoritative home**: the one document in a comparison scope that owns a
  fact. Other documents point to it.
- **Volatile value**: a value that changes independently of the document that
  states it: a current version, a deployment state, a commit identifier, a
  count.
- **Pointer**: text or a link that sends the reader to where an answer lives.
- **Exception**: an explicit, scoped, reasoned statement that a requirement
  does not apply to identified content.
- **Comparison scope**: the documents among which ownership and duplication are
  evaluated.
- **Checker**: a tool that evaluates documents against this specification.
- **Finding**: a checker's report that content does not, or may not, satisfy a
  requirement.

## Document kinds

`KIND-1` Every document in the documentation set MUST have exactly one kind,
taken from the table below.

`KIND-2` A kind is assigned by a declaration in the document or by a mapping
from the document's path. A declaration is the `kind` key of the document's
YAML frontmatter, with a kind name as its string value. The form of a mapping
belongs to the documentation set's configuration and is outside this
specification. When a declaration and a mapping both apply, the declaration
MUST take precedence.

`KIND-3` A document whose frontmatter cannot be parsed, or whose `kind` value
is not a kind name, has no kind. It MUST NOT receive a kind from a mapping
while the declaration is broken.

`KIND-4` `generated` MUST be assigned only by mapping. A `generated`
declaration is invalid, and the document has no kind. A document cannot exempt
itself from checking.

`KIND-5` Declaring a kind accepts its content contract: the document answers
the kind's question, holds the content the kind is for, and points to content
that belongs elsewhere instead of holding it. A declaration does not establish
that any statement in the document is correct or current.

| Kind | Question answered | Contents | Content that belongs elsewhere |
| --- | --- | --- | --- |
| `readme` | What is this, and how do I start? | Purpose, quick start, documentation links | Field catalogs, architecture detail, current version snapshots |
| `howto` | How do I do this? | Steps, commands, expected results | Design arguments, complete option references |
| `reference` | What exists? | Definitions, tables, accepted values | Procedures, project history |
| `runbook` | How do I respond to an incident? | Inspection commands, recovery steps | Current deployment state and version snapshots |
| `agents` | What does a coding agent need to work here? | Commands, conventions, boundaries | Project history, volatile facts, conversation remnants, facts owned by other documents; link to their sources |
| `adr` | Why was this design chosen? | Context, decision, tradeoffs, date | No additional kind-specific restriction |
| `plan` | What work comes next? | Proposed stages, dates, progress | Authoritative contracts for completed behavior |
| `changelog` | What happened? | Dated changes, versions, commits | No additional kind-specific restriction |
| `generated` | What did the generator produce? | Generator-owned content | A frontmatter declaration cannot grant this exemption |

`readme`, `howto`, `reference`, `runbook`, and `agents` are long-lived
documents. `adr` and `changelog` are dated records. `plan` is a proposal. A
generated document is owned by its generator: the requirements of this
specification apply to the generator's templates rather than to its output,
while the output remains a link target and a possible authoritative home.

`KIND-6` A long-lived document MUST address its reader directly and stand on
its own. It MUST NOT address whoever requested it, report what that requester
asked for or approved, or narrate how the document or the work it describes
was produced, including attempts and options that the result replaced. It MAY
describe earlier behavior that the reader can still meet, such as a deprecated
option or a migration from an earlier version.

`KIND-7` A `howto`, `reference`, or `runbook` MUST NOT argue for a design
choice. A sentence of rationale that a step or definition needs in order to be
understood MAY stay. A heading or section that explains why a choice was made
belongs in an `adr`, and the procedure or contract points to it.

`KIND-8` A `howto` or `runbook` SHOULD reach its procedure before extended
background, and SHOULD place troubleshooting and recovery after the main flow.

Classify a product requirements document or a specification by the
responsibilities in the table. Before implementation, requirements and feature
specifications are a `plan`. A maintained specification of behavior that must
hold now is a `reference`, including protocol and file-format specifications.
Design decisions and tradeoffs are an `adr` that either document points to.
Once behavior ships, its definitions move from the plan to a reference page,
and the plan points to that page. `prd` and `spec` describe content, not kinds.
This version of the specification defines no mechanism for additional kinds.

## Facts and authority

`FACT-1` Within a comparison scope, each fact MUST have exactly one
authoritative home. Other documents MUST point to that home instead of
restating the fact. A document MAY orient the reader with a summary, but a
summary that reproduces the definitions of its target is a restatement, even
when it links to the target.

`FACT-2` The authoritative home follows from responsibilities: definitions and
accepted values belong to a `reference` or to generated output, decisions to
an `adr`, dated changes to a `changelog`, and procedures to a `howto` or
`runbook`. A document MAY declare `canonical: true` in its frontmatter to own
every fact it states within its scope; the declaration applies to the whole
document and takes precedence over responsibilities. Two candidate homes that
neither a declaration nor responsibilities separate are a defect of the
documentation set, and its authors resolve it.

`FACT-3` Ownership and duplication are evaluated within a comparison scope. A
documentation set MAY partition itself into domains so that unrelated
vocabularies are not compared. Documents in different languages are not
compared: a translation is not a competing home. A document MAY declare its
language with a `lang` key in its frontmatter, such as `lang: en`; the
declaration takes precedence over detection.

`FACT-4` A `plan` MUST NOT be the authoritative home of a definition for
behavior that has shipped. When behavior ships, its definitions move to a
`reference`, and the plan points to that page.

`FACT-5` A dated record MAY preserve the values it observed at its date. Those
values are history, not current facts, and the record is not a competing home
for them.

`FACT-6` A claim that evaluates or compares SHOULD identify its evidence: a
measurement, a source, or a demonstration near the claim.

## Pointers and changing information

`POINTER-1` A long-lived document MUST NOT state a volatile value as the
current state. It MAY state a requirement or a range, such as a minimum
version; a historical fact, such as the release that introduced a feature; or
an example identified as one. For the current value it points to the source
that answers the question: a manifest, a release record, a command, a
dashboard. A requirement replaces a snapshot only when the requirement holds
independently; an observed value does not establish one.

`POINTER-2` A pointer MUST identify where the answer lives: a file, a symbol, a
section, a command, or a document. A pointer to a repository root, to an
unnamed source, or to an arbitrary directory does not. A directory is an
acceptable target when it is a catalog whose index or naming convention tells
the reader how to select an entry.

`POINTER-3` A link to a file in the repository MUST resolve: the target
exists, and a fragment names an anchor in the target document.

## Exceptions

`EXCEPTION-1` An exception MUST be written in the document it affects, as an
HTML comment of the form `<!-- seiso: allow CODE, CODE -- reason -->`. Each
code is a complete finding identifier of the checker in use, never a prefix
or a family, and the reason is one a reviewer can evaluate. On its own line
before a block, the comment applies to that block; inside a paragraph, list
item, or table cell, it applies to that unit; written as `allow-file` after
the frontmatter and before the first content block, it applies to the whole
document. This version defines no code for a documentation set that no
checker evaluates. Such a set cannot except a requirement, so it conforms only
by satisfying every MUST requirement outright.

`EXCEPTION-2` An exception records a judgment. It does not make the excepted
content correct, and it does not complete a check that could not run. An
exception without a reason, or one that names an unknown or incomplete code,
is itself a violation.

`EXCEPTION-3` An exception that no longer covers any finding SHOULD be
removed, so that it cannot hide a later finding in the same scope.

## Conformance

`CONFORMANCE-1` A documentation set conforms to a version of this
specification when every document has a kind, every MUST requirement is
satisfied or covered by an exception, and its authors have reviewed what no
checker established: the requirements listed below under judgment, and the
parts of partially established requirements that the checker in use does not
cover. A claim of conformance MUST name the specification version. It MAY
name the checker used and the requirements that checker covered; a checker's
pass is evidence for those requirements only.

Requirements differ in what a checker can establish from the documents alone:

| Established by | Requirements |
| --- | --- |
| A checker, completely | `KIND-1`, `KIND-2`, `KIND-3`, `KIND-4`, `FACT-3`, `POINTER-3`, `EXCEPTION-1`, `EXCEPTION-3` |
| A checker in part, the remainder by judgment | `KIND-6`, `KIND-7`, `KIND-8`, `FACT-1`, `FACT-2`, `FACT-4`, `FACT-6`, `POINTER-1`, `POINTER-2` |
| Judgment | `KIND-5`, `FACT-5`, `EXCEPTION-2` |

`CONFORMANCE-2` A checker MUST state which version of this specification it
checks and, for each requirement, whether it establishes the requirement
completely, in part, or not at all.

`CONFORMANCE-3` The strength of a finding MUST NOT exceed its evidence. A
checker distinguishes a finding that states a mechanically established fact,
such as a missing link target; a finding that identifies a breach of a content
contract, such as a design heading in a how-to; and a finding that describes
an observed feature and suggests a judgment, such as prose that resembles a
snapshot of a changing value.

`CONFORMANCE-4` When a checker cannot complete a check, because an input was
unreadable or a required document was unavailable, it MUST report the check as
incomplete and MUST NOT present the result as a pass.

`CONFORMANCE-5` Where a requirement needs a judgment, such as which of two
documents owns a fact, a checker MUST name the decision to be made rather than
make it silently.

`CONFORMANCE-6` A checker's thresholds, heuristics, rule availability,
configuration format, command-line interface, and output formats are
implementation choices. They are not requirements of this specification, and
passing a checker's selected checks is evidence only for the requirements
those checks cover.

## Versioning

This specification is versioned independently of any checker, as
MAJOR.MINOR.PATCH. A released version does not change: every later change,
however small, is released as a new version, and the
[changelog](CHANGELOG.md) records each one.

From 1.0.0, a change that can make a conforming documentation set
non-conforming increments MAJOR: a new or strengthened MUST, a removed kind, a
changed kind contract, changed frontmatter or exception semantics, or a
renumbered identifier. Any other change to the requirements, such as an added
kind or a new SHOULD or MAY, increments MINOR. A change that leaves every
requirement as it was, such as a clarification, a corrected example, or
explanatory text, increments PATCH. Before 1.0.0, every change to the
requirements increments MINOR, including one that can make a conforming set
non-conforming, and any other change increments PATCH.

A checker states the version it checks, and a documentation set states the
version it claims (`CONFORMANCE-1`, `CONFORMANCE-2`). A claim names a released
version, never the unreleased text.

## Relation to other conventions

[Diátaxis](https://diataxis.fr/) organizes documentation by the reader's need
into tutorials, how-to guides, reference, and explanation. `howto` and
`reference` correspond to its how-to and reference types. Diátaxis explanation
has no single counterpart: an `adr` records one decision with its date, and
explanatory prose that a reader needs in order to use a contract stays in the
reference that owns it. `readme`, `runbook`, `agents`, `plan`, `changelog`,
and `generated` describe responsibilities that Diátaxis does not name. The two
are compatible: a Diátaxis site can declare a kind for each page. This
specification adds what Diátaxis leaves to the author, namely one home per
fact, pointers to changing information, recorded exceptions, and a definition
of conformance.

This specification does not define the internal format of any kind.
[Keep a Changelog](https://keepachangelog.com/) can shape a `changelog`, and
Nygard-style decision records can shape an `adr`; the convention constrains
what each kind is responsible for, not how its sections are laid out.
