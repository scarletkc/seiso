---
kind: reference
---

# Worked examples

These examples read the requirements of the
[Seiso Convention Specification](convention.md) against one small
documentation set. Each example shows a document or an excerpt, names the
requirement it exercises, and says whether a checker can establish the result
from the documents alone or whether the authors decide. The examples explain;
the specification is authoritative. Where an exception names a code, the code
is one that [seiso](../README.md) reports, because exception codes belong to
the checker in use.

## The documentation set

```text
README.md
AGENTS.md
CHANGELOG.md
docs/
  install.md
  reference/
    settings.md
  decisions/
    0001-file-based-cache.md
  plans/
    remote-cache.md
  api/
    generated/
      cli.md
```

`docs/install.md`, `docs/reference/settings.md`, and the decision record
declare their kinds in frontmatter. The other documents receive kinds from a
path mapping in the project's configuration:

| Path | Kind |
| --- | --- |
| `README.md` | `readme` |
| `AGENTS.md` | `agents` |
| `CHANGELOG.md` | `changelog` |
| `docs/plans/**` | `plan` |
| `docs/api/generated/**` | `generated` |

The mapping's syntax belongs to the checker; seiso reads it from `[[kinds]]`
entries, as the [configuration reference](../docs/reference/configuration.md#kinds-and-domains)
describes. Every document in the set therefore has exactly one kind, and
`KIND-1` is satisfied.

### Precedence between a declaration and a mapping

`docs/plans/remote-cache.md` is mapped to `plan`. If it also declares
`kind: adr`, the declaration wins and the document is an `adr` (`KIND-2`). A
checker establishes this.

If its frontmatter is malformed, the document has no kind, and the mapping
does not rescue it (`KIND-3`):

```markdown
---
kind: [plan
---

# Remote cache
```

`docs/api/generated/cli.md` is mapped to `generated`. If the file itself
declares `kind: generated`, the declaration is invalid, the document has no
kind, and it is checked like any other document with no kind (`KIND-4`). The
mapping alone would have exempted it; the declaration takes that away. A
checker establishes this.

## A how-to

`docs/install.md` satisfies its contract: steps, commands, and an expected
result, with definitions left to the reference that owns them.

````markdown
---
kind: howto
---

# Install

Install the release for your platform:

```sh
cargo install example
```

`example` requires Rust 1.75 or later. It reads its settings from
`example.toml`; the [settings reference](reference/settings.md) defines them.

## Verify

Run `example --version`. The command prints the installed version.
````

The minimum Rust version is a requirement, which `POINTER-1` permits. The
verification step describes what the command prints without recording the
value. The settings are referred to, not restated (`FACT-1`).

### Rationale a step needs

One sentence that a reader needs in order to follow a step stays in the how-to
(`KIND-7`):

```markdown
Set `cache.dir` to a local disk. The cache is content-addressed, so a shared
network path gains nothing and adds latency.
```

A section that argues for the design breaks `KIND-7`:

```markdown
## Why the cache is file-based

Redis and SQLite were evaluated before files were chosen. Both add a service
or a dependency, and the workload fits within the latency target on a
filesystem.
```

The heading is a form a checker can recognize. The argument belongs in
`docs/decisions/0001-file-based-cache.md`, and the how-to points to it. Whether
three paragraphs of background under a neutral heading such as
`## Before you start` are what the reader needs or a delay to the procedure
(`KIND-8`) is a judgment; a checker can observe the length and suggest.

### Volatile values

This sentence breaks `POINTER-1` in a how-to:

```markdown
The current release is 2.4.1.
```

The repair that preserves its meaning points to where releases are recorded:

```markdown
Releases are listed in the [changelog](../CHANGELOG.md).
```

A requirement is a different statement, not a rewrite of the same one. This
satisfies `POINTER-1`, but only when the minimum is established on its own,
for example by the compatibility policy or the test matrix:

```markdown
`example` requires 2.4 or later.
```

Turning an observed release into a minimum to satisfy the convention invents
a requirement; a checker cannot tell the two apart, and the authors must.

`Tested with 2.4.1` is a boundary case: a compatibility statement, a
historical fact, or a snapshot of the test matrix. A checker can observe the
version label and ask; the authors decide. If the exact value must stay, the
sentence carries an exception with the reason:

```markdown
<!-- seiso: allow STL004 -- The compatibility example names the exact version the test matrix pins. -->
Tested with 2.4.1 on Linux and macOS.
```

### Conversation remnants

This breaks `KIND-6`, because a long-lived document addresses its reader, not
the person who asked for it:

```markdown
As you asked, the verification step is now included below.
```

A checker recognizes some of these phrases. A paragraph that narrates the
document's own production in other words is a judgment.

A request also survives without a requester phrase. A review of
`docs/install.md` asked that the verification step not pin a release number,
and the request became part of the step:

```markdown
Run `example --version`. The command prints the installed version, without
comparing it to a pinned release number.
```

The final clause reports what the reviewer asked for, with the reviewer left
out, and `KIND-6` excludes that report whether or not it names the requester.
A reader who never saw the review learns what the step does not do, about an
option nobody offered them. The verification step above states what the
command does and nothing else.

A negative statement stays when it tells the reader something they would
otherwise assume or do. A reader of a Markdown checker may expect external
links to be fetched and grant the check network access; this sentence states
the constraint and its consequence:

```markdown
`example` does not fetch external links, so a check runs without network
access.
```

`without` and `does not` are ordinary contract prose, so a checker cannot
separate the two sentences. A reviewer who never saw the conversation
separates them by asking: without the sentence, would the reader assume or do
the excluded thing?

### Abandoned attempts

Narrating how the work was produced breaks `KIND-6` even when nobody asked
for anything. This sentence in `docs/install.md` describes a cache the reader
can never meet:

```markdown
An early build kept the cache in SQLite; files replaced it before the first
release.
```

The repair deletes the sentence. The decision record holds the alternatives
(`KIND-7`), and the how-to points to it where a step needs the reason.
Earlier behavior that the reader can still meet is current content, which
`KIND-6` permits:

```markdown
`example` 2.0 does not read caches written by 1.x. Delete `.example_cache`
after upgrading.
```

## A reference page

`docs/reference/settings.md` is the authoritative home of the settings
(`FACT-2`). Defaults are part of the contract the page owns, not snapshots of
values that change without the page.

```markdown
---
kind: reference
---

# Settings

`example.toml` at the repository root supplies these settings.

| Setting | Meaning | Default |
| --- | --- | --- |
| `cache.dir` | Directory for content-addressed parse results | `.example_cache` |
| `cache.max-age` | Days before an unread entry is removed | `30` |
| `include` | Glob patterns of files to check | `**/*.md` |
| `exclude` | Glob patterns to skip | none |
| `preview` | Enable preview checks | `false` |
```

### A restatement in the README

This README section breaks `FACT-1`, even though it links to the reference,
because it reproduces the definitions:

```markdown
## Configuration

- `cache.dir`: directory for parse results
- `cache.max-age`: days before an entry is removed
- `include`: files to check
- `exclude`: files to skip
- `preview`: enable preview checks

See the [settings reference](docs/reference/settings.md) for details.
```

A checker can establish that the two blocks define the same keys. This
satisfies the requirement:

```markdown
## Configuration

`example.toml` holds the settings; the
[settings reference](docs/reference/settings.md) defines them.
```

An example configuration in the README is an example, not a definition, and
`FACT-1` permits it:

````markdown
```toml
[cache]
dir = ".example_cache"
```
````

Whether a two-sentence summary of what the cache settings do is orientation or
restatement is a judgment.

### Two homes at the same rank

If `docs/reference/cache.md` also defines `cache.dir` and `cache.max-age`, two
references own the same facts and neither outranks the other. The set has a
defect under `FACT-2`. A checker reports the tie and names the decision
(`CONFORMANCE-5`). The repair has two parts: the authors choose the owner,
declaring `canonical: true` in its frontmatter if the kinds alone do not make
it the owner, and then replace the definitions in the other page with a
pointer to it. The declaration settles who owns the facts; the remaining copy
still breaks `FACT-1` until it becomes a pointer.

### A procedure in a reference

A `## Rotate the cache` section with numbered stop, delete, and start steps
answers "how do I do this", which is the `howto` question. The reference's
contract excludes procedures (`KIND-5`). A checker with section
classification can suggest the mismatch; the contract, not the classifier,
decides.

## A decision record

`docs/decisions/0001-file-based-cache.md` is a dated record.

```markdown
---
kind: adr
---

# 0001: Store parse results as files

Date: 2026-03-14

## Context

Checks parse every document on each run. At the time of this decision the
corpus held 1,240 documents and a cold run took 9 seconds on the CI runner.

## Decision

Store content-addressed parse results in the directory `cache.dir` names.
Entries are keyed by the hash of the source, so a changed file misses the
cache and is parsed again.

## Consequences

Moving a file keeps its cache entry. Redis and SQLite were rejected: both add
a service or a dependency for a workload that a filesystem serves within the
latency target.
```

The document count and the duration are values observed on the date of the
record, which `FACT-5` permits. The rejected alternatives are the content
`KIND-7` sends here. `cache.dir` is a pointer to the setting the reference
owns, not a second definition.

If the record reproduced the settings table, the reference would own the
definitions by responsibility and the record would be restating them
(`FACT-1`). A checker can establish the match. Whether the how-to should point
to this record for the cache directory's current default is a judgment with
one answer: the record holds history, and the reference holds the current
contract (`FACT-5`).

## A plan

`docs/plans/remote-cache.md` receives `plan` from the mapping.

```markdown
# Remote cache

Stage 1, proposed for the fourth quarter of 2026: add `cache.remote` and
`cache.token` so that CI runners share parse results.

- `cache.remote`: URL of the shared cache service.
- `cache.token`: bearer token, read from the environment.

Stage 2: measure the hit rate on the CI runner before making the remote cache
the default.
```

While the remote cache is unshipped, the plan is the only home of these two
settings, and stages and dates are the plan's own content. Once stage 1 ships
and the settings reference defines `cache.remote` and `cache.token`, the plan
breaks `FACT-4` by keeping the definitions. A checker can establish that a
plan and another document define the same keys. This satisfies the
requirement:

```markdown
Stage 1 shipped. `cache.remote` and `cache.token` are defined in the
[settings reference](../reference/settings.md).
```

## Pointers

Each of these breaks `POINTER-2`:

```markdown
See the source for the accepted values.

Read the [documentation](https://github.com/example/example).

The rule pages are in [docs/](docs/).
```

Each of these satisfies it:

```markdown
`Settings` in [`src/config.rs`](../src/config.rs) lists the accepted values.

Read the [installation guide](docs/install.md).

The rule pages are in [docs/rules/](docs/rules/), one file per rule code.
```

The last pointer targets a directory. `POINTER-2` permits it because the
directory is a catalog whose naming convention selects an entry. A checker
cannot tell a catalog from an arbitrary directory; the documentation set
records which directories are catalogs, and seiso reads that list from
`catalog-dirs`.

This breaks `POINTER-3` when the target's heading is `## Settings`:

```markdown
The [options](reference/settings.md#options) are listed there.
```

A checker establishes missing files and anchors completely for links within
the set. It does not fetch external URLs.

## Exceptions

`docs/api/generated/cli.md` is written by the release build and is absent
from a fresh checkout. This exception satisfies `EXCEPTION-1`:

```markdown
<!-- seiso: allow LNK001 -- docs/api/generated/cli.md is written by the release build. -->
The [CLI reference](api/generated/cli.md) lists every command.
```

Each of these is itself a violation (`EXCEPTION-2`): the first names a family
instead of a code, the second gives no reason.

```markdown
<!-- seiso: allow LNK -- The release build writes this page. -->

<!-- seiso: allow LNK001 -->
```

Once the generated page is committed, the exception covers nothing, and
`EXCEPTION-3` says to remove it. A checker establishes that the exception
suppressed no finding.

If the checker could not read `docs/reference/settings.md`, an exception on a
link into that file does not complete the link check. The checker reports the
check as incomplete (`CONFORMANCE-4`), and the documentation set cannot claim
the requirement on the strength of the exception (`EXCEPTION-2`).

## Judgments

No checker establishes these from the documents alone; a conformance claim
covers them through review (`CONFORMANCE-1`):

- Whether a negative clause in a long-lived document tells the reader about
  something they would otherwise assume or do, or answers an option that only
  the conversation behind the document raised (`KIND-6`).
- Whether a paragraph in a how-to is the sentence of rationale a step needs
  or the beginning of a design argument (`KIND-7`).
- Whether two paragraphs that explain the same behavior in different words are
  one fact with two homes or two explanations for two audiences (`FACT-1`).
- Whether a statement such as `Tested with 2.4.1` is a requirement, history,
  or a snapshot (`POINTER-1`).
- Whether the evidence beside a comparative claim supports it (`FACT-6`). A
  checker sees that a link or a measurement is present; it does not read the
  benchmark.
- Whether an exception's reason justifies the exception (`EXCEPTION-2`).
