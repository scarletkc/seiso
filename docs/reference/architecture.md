---
kind: reference
---

# Architecture and execution

The root Cargo package exposes a library and CLI. The modules below separate
input loading, analysis, and command rendering.

| Module | Responsibility |
| --- | --- |
| [`main`](../../src/main.rs), [`commands`](../../src/commands.rs) | CLI arguments, command orchestration, rendering and writes |
| [`workspace`](../../src/workspace.rs) | Shared discovery, policy resolution, scoped reads, parsing, and input errors |
| [`analysis`](../../src/analysis.rs) | Check execution, suppression application, report selection, and fix proposals |
| [`config`](../../src/config/mod.rs) | Configuration discovery, explicit inheritance, and path policy |
| [`md`](../../src/md/mod.rs) | Content-derived document model and original source mappings |
| [`paths`](../../src/paths.rs) | Path normalization, local destination parsing, and filesystem target status |
| [`index`](../../src/index/mod.rs) | Workspace facts, anchors, comparison domains, and resolved links |
| [`sections`](../../src/sections.rs) | Heuristic section roles with source evidence |
| [`rules`](../../src/rules/mod.rs) | Rule registry, rule functions, suppression states, and safe edit construction |
| [`diagnostics`](../../src/diagnostics/mod.rs) | Diagnostics, locations, related evidence, and fix data |
| [`cache`](../../src/cache/mod.rs) | Content-addressed parse storage |

## Loading and command scope

`workspace::load` supplies the shared snapshot. Its loading scope is independent
of rule execution:

| Command | Inputs read | Rule execution |
| --- | --- | --- |
| `parse` | Selected documents | None |
| `check` and editor hooks | Selected documents, plus workspace documents when an included policy can enable an index-dependent rule | Selected single-file checks and required cross-file checks |
| `policy` | Included workspace documents | None; declarations and policy are inspected |
| `policy --evaluate` | Included workspace documents | Checks run to establish suppression outcomes |
| `index --dump` | Included workspace documents | None |

An unqualified check selects every included document. A path check with only
single-file rules reads the requested sources; an unrelated invalid UTF-8 file
does not invalidate it. Discovery still considers included policies before
deciding whether an index is needed. An enabled cross-file rule elsewhere may
produce an incoming diagnosis related to the selected file, so dependency
scope cannot be inferred from the selected files alone.

Single-file rules run on selected files. Cross-file diagnostics are reported
when either their primary or related location is selected. For complete inputs,
a path check therefore agrees with the corresponding subset of a full check.
Read, configuration, or ignore errors in required inputs make the check
incomplete; unrelated errors outside a local check's required scope do not.

Stdin replaces one named document for that invocation and can supply a new
path within the workspace. It participates in analysis without writing a file.

## Document and index data

Rules consume the seiso document model through ordinary functions. The model
records frontmatter,
sections, blocks, sentences, fragment kinds, raw destinations, and comments.
Every fragment retains a byte range into the original source. Display columns
count Unicode characters; ambiguous or synthesized mappings retain their
containing range and are marked inexact.

The rule registry owns execution phase, stability, applicable kinds, fragment
requirements, and filesystem/index dependencies. `requires_index` drives
workspace dependency loading. Rule execution reports completion separately from
diagnostics so SUP002 can distinguish a completed empty result from missing inputs.
Final diagnostics are sorted deterministically before rendering.

The index combines parsed facts with current kind, language, domain, and path
policy. Generated files remain index sources but run no rules. Files without a
valid kind receive only the kind-independent checks. Excluded and non-Markdown
files may be checked for physical existence but do not supply Markdown anchors.

Section classification is computed from the parsed document when requested by
`parse` or a heuristic rule. It is separate from the cached parse facts and
does not assign a document kind. The [section reference](sections.md) defines
its roles, evidence, and boundaries.

## Links

`paths::local_link_targets` lists a destination's candidate targets: the
written path, then, for a document a `[[sites]]` entry matches, the page
sources of its route. `paths::select_target` picks the first that exists, and
`paths::local_link` also parses the anchor. Percent decoding, scheme detection,
workspace containment, normalization, and route candidates are shared by
file-existence and index rules. A relative path starts at the source directory;
a leading slash starts at the workspace root. Local filesystem inspection and frozen Git
inventories supply physical existence through their respective adapters.

Existence and anchor knowledge are distinct. A file outside the parsed sample
can exist while its anchors remain unknown. External URLs, template values,
outward paths, and unresolved inputs do not justify a missing-target diagnosis.
GitHub-style heading slugs include duplicate suffixes; HTML `id` and anchor
`name` attributes are indexed. Undefined reference labels remain ordinary
CommonMark text. [LNK001](../rules/LNK001.md) and [LNK002](../rules/LNK002.md) define the
diagnostic boundaries.

## Suppression and fixes

`policy` inspects declarations without claiming rule outcomes: enabled valid
codes have state `not_evaluated`; invalid and disabled codes retain those
states. `policy --evaluate` records actual active, stale, or incomplete states.
[SUP001](../rules/SUP001.md) owns syntax and scope precedence;
[SUP002](../rules/SUP002.md) owns completion and unused-code rules.

Suppression codes are carried as structured diagnostic data, so wording changes
cannot change which codes a fix removes. Fix mode stops after the initial
analysis when no safe edits exist. A real edit plan requires a fresh analysis
to confirm the inputs and diagnostics, source equality immediately before
writing, and a check of the result. Incomplete checks cannot authorize edits.

## Cache

Only content-derived parse data is cached. Needed inputs are read and hashed
on each invocation; modification times do not determine freshness. Kind,
domains, filesystem links, rule results, and suppression outcomes are resolved
again. Moving identical content therefore uses its new policy and destinations.

Missing, corrupt, incompatible, and unwritable entries fall back to parsing.
Writes use temporary files and atomic persistence; concurrent processes do not
share mutable parsed state. `--no-cache` bypasses reads and writes. Equivalent
inputs must produce identical diagnostics with a cold, warm, or disabled cache.

The cache directory holds its own `.gitignore` and `CACHEDIR.TAG`. Because
entries are content-addressed, edited sources and earlier seiso versions leave
entries that are never read again. At most once a day, a command that writes
to the cache also removes entries older than `MAX_ENTRY_AGE` in
[`src/cache/mod.rs`](../../src/cache/mod.rs); a removed entry only costs a
later parse.
