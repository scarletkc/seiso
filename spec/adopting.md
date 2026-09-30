---
kind: howto
---

# Adopt the convention

Apply the [Seiso Convention Specification](convention.md) to an existing
repository. Each step names the requirements it satisfies; the
[worked examples](examples.md) show each one on a small documentation set. A
checker is optional, with one consequence described under
[Record exceptions](#record-exceptions): without one, the documentation set
cannot except a requirement.

## Assign a kind to every document

List the Markdown files the conformance claim will cover, leaving out vendored
and test material. Give each file one kind from the
[kinds table](convention.md#document-kinds) and declare it in frontmatter:

```markdown
---
kind: howto
---
```

For files that cannot carry frontmatter, and for whole directories, record a
path mapping in the project's configuration instead. Frontmatter wins over a
mapping, and a broken declaration leaves the file without a kind (`KIND-1` to
`KIND-4`).

## Move each fact to its home

For each definition, accepted value, default, or decision that appears in more
than one document, choose the home by responsibility: a reference for
definitions, a decision record for decisions, a changelog for dated changes.
Replace every other copy with a link. Where responsibilities do not decide, or
the owner must be a document they would not choose, add `canonical: true` to
the owner's frontmatter; it takes precedence over responsibilities (`FACT-1`,
`FACT-2`, `FACT-4`).

## Replace volatile values with pointers

In each long-lived document, look for current versions, deployment states,
commit identifiers, and counts. Replace each one with a link to the manifest,
release record, or command that answers the question, or move it to a dated
record as an observation with its date. State a requirement or a supported
range only where that requirement is established on its own; a value that was
observed does not become a minimum by being rewritten. Make every pointer name
a file, symbol, section, or document (`POINTER-1`, `POINTER-2`).

## Record exceptions

Where a requirement does not apply and the checker in use reports a finding
for the content, write the exception in the document, with that finding's
complete code and a reason a reviewer can evaluate:

```markdown
<!-- seiso: allow LNK001 -- The release build writes this page. -->
```

Remove an exception once it covers nothing (`EXCEPTION-1` to `EXCEPTION-3`).

Everywhere else the content has to satisfy the requirement, because the
specification defines no other form. That covers a documentation set that no
checker evaluates, and content for which the checker reports nothing, such as
a judgment requirement its rules do not reach. Specification identifiers such
as `KIND-7` are not codes; seiso reports them as invalid. Whether they should
become codes is an open question in the
[roadmap](../docs/design/roadmap.md#convention-specification).

## Instruct coding agents

Add the following to `AGENTS.md`, `CLAUDE.md`, or the equivalent instruction
file, so that the convention applies while a document is written rather than
after. The snippet summarizes; the specification is authoritative. Replace
`VERSION` with the version you adopted from the [changelog](CHANGELOG.md), so
that the agents read the requirements you reviewed rather than those of a
later version.

```markdown
## Documentation convention

Markdown in this repository follows the Seiso Convention VERSION:
https://seiso.fog.moe/VERSION/convention

- Every document has one `kind`, declared in frontmatter or assigned by the
  configured path mapping: readme, howto, reference, runbook, agents, adr,
  plan, or changelog; `generated` is assigned only by mapping. Hold only
  what that kind is for; a how-to gives steps, a reference gives
  definitions, an ADR gives the reasons.
- Each fact has one home. Link to it instead of restating it.
- Long-lived pages state requirements and point to sources. They do not
  record the current version, deployment state, commit id, or count.
- A pointer names a file, symbol, section, or document, never "the source"
  or a repository root.
- Do not address the person who asked for the document or describe how it
  was written.
- Where the checker in use reports a finding that does not apply, write
  `<!-- seiso: allow CODE -- reason -->` with that finding's complete code
  and a reason a reviewer can evaluate. Without a checker, or without a
  finding to name, satisfy the requirement instead.
```

## Run a checker

seiso is the reference implementation. From the repository root:

```sh
seiso init
seiso check
```

The [README](../README.md#install) lists the install commands, and
[checking documents](../docs/guides/checking.md) explains the configuration
and output. [How seiso checks the convention](../docs/reference/convention.md#requirement-coverage)
maps each requirement to the rules that cover it completely, in part, or not
at all; a passing check is evidence for those requirements only
(`CONFORMANCE-1`, `CONFORMANCE-6`).

## Claim conformance

Before making the claim, review what no checker established: the requirements
the specification lists under judgment, and the parts of partially covered
requirements that the checker's rules do not reach, such as paraphrased
restatements under `FACT-1` or rationale in prose under `KIND-7`. Then state
the specification version and, when one is used, the checker, for example in
the README: "Documentation follows the Seiso Convention VERSION, checked with
seiso." Link the claim to that version's copy, as the agent snippet does
(`CONFORMANCE-1`).
