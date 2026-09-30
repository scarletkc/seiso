---
kind: changelog
---

# Changelog

Changes to the [Seiso Convention Specification](convention.md). The
specification is versioned independently of the seiso crate, whose
[releases](https://github.com/scarletkc/seiso/releases) carry their own notes;
its [Versioning](convention.md#versioning) section defines the numbers. Each
released version is published at the `spec-v<version>` tag of this
repository. This file follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

## 0.1.0 - 2026-10-01

### Added

- The specification, restating the convention that seiso has
  documented since 0.1.0 as requirements with stable identifiers: document
  kinds (`KIND-1` to `KIND-8`), facts and authority (`FACT-1` to `FACT-6`),
  pointers and changing information (`POINTER-1` to `POINTER-3`), exceptions
  (`EXCEPTION-1` to `EXCEPTION-3`), and conformance for documentation sets
  and checkers (`CONFORMANCE-1` to `CONFORMANCE-6`).
- A table of which requirements a checker can establish completely, in part,
  or not at all.
- A versioning section that defines which changes increment MAJOR, MINOR,
  and PATCH, and that a released version does not change.
- A section on the relation to Diátaxis and to the formats of changelogs and
  decision records.
- [Worked examples](examples.md) covering a how-to, a reference page, a
  decision record, and a plan, with kind assignment by frontmatter and by
  mapping, boundary cases, exceptions, and judgments.
- An [adoption guide](adopting.md) with an instruction snippet for coding
  agents.
