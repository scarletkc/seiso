---
kind: howto
---

# Check Markdown

Run checks from the document workspace:

```sh
seiso init
seiso check
```

`seiso init` writes `seiso.toml` at the repository root. Review its suggested
exclusions, kind mappings, and documentation site entries. Add `kind`
frontmatter to documents that need a different role. Use `seiso rule KND001`
for an example.

Stable rules are enabled by default. Add `--preview` or `preview = true` in the
configuration to opt into selected preview rules. Preview rules are
experimental and can report false positives; the
[evaluation records](../README.md#evaluation) show how each one performed, so
review them before using a preview rule as a gate. `seiso rule <CODE>` shows a
rule's status; `seiso policy` shows the rules enabled for each file. The
[evaluation policy](../evaluation/policy.md) records the promotion criteria.

Language-specific rules use each sentence's detected language. Declare `lang`
in frontmatter when detection picks the wrong one; see
[document language](../reference/configuration.md#document-language).

## Select files and rules

```sh
seiso check docs README.md
seiso check --select KND,LNK,SUP
seiso check --preview --extend-select VOX001
seiso rule VOX001
```

Paths are relative to the calling directory. Reported filenames are relative to
the workspace root. Explicit paths still respect `.gitignore`, `include`, and
`exclude`; the nearest configuration determines each file's policy. When a
named path selects no document, the check prints the reason to stderr: the file
is not Markdown, `.gitignore` or the configuration excludes it, or the
directory contains no Markdown files. A selector that names only preview rules
also gets a notice when preview is not enabled.

With only single-file rules enabled, a path check reads the selected sources.
When an included file's policy can enable a cross-file rule, checking loads the
included workspace documents and reports a diagnosis when its primary or
related location is selected. Checking a renamed heading's file can therefore
report a broken anchor in a document that links to it. Unrelated unreadable
documents do not block a check that does not need their content.

Use `seiso check --help` for command options and `seiso rule --all` for the
implemented rules, their examples, and exceptions. `--select` replaces the
configured selection; `--extend-select` adds to it.
Only implemented families and rule codes are accepted. The
[configuration reference](../reference/configuration.md) defines inheritance, precedence,
kind mappings, domains, documentation sites, and rule selection.

## Check unsaved content

Send UTF-8 Markdown through stdin and name its workspace path:

```sh
seiso check --stdin-filename docs/guide.md < draft.md
```

The content replaces that file for this check. Relative links and configuration
use the named path; the command leaves the file on disk unchanged. The path may
be new, but it must be inside the workspace and included by its policy.

## Consume results

```sh
seiso check --output-format concise
seiso check --output-format json
seiso check --output-format sarif > seiso.sarif
seiso check --output-format github
seiso check --statistics
seiso policy > policy.json
seiso policy --evaluate > evaluated-policy.json
seiso index --dump > index.json
```

Text includes source excerpts. Concise output puts each diagnostic and its
suggestion on one line. JSON is a sorted array of diagnostics; tool errors go to
stderr. Each JSON diagnostic's `url` links to its rule explanation at the
release tag matching the binary's package version. SARIF exposes the same link
in each rule's `helpUri`.

Policy JSON includes effective settings, kind resolution, enabled rules,
configuration exclusions, and suppression records. The index dump includes
effective kind, language, domain, anchors, and outgoing links.

`policy` inspects declarations without running rules: valid enabled suppression
codes are `not_evaluated`; invalid or disabled codes retain their states. Add
`--evaluate` to determine actual active, stale, and incomplete outcomes.
`index --dump` builds the index without running checks.

SARIF includes primary and related locations and available safe fixes. GitHub
output uses workflow annotations with paths relative to the Git repository root.
Other formats report paths relative to the workspace. GitHub uses the repository
containing the workspace as the base for primary and related locations; outside
a repository, it reports absolute paths.
With `--statistics`, JSON becomes an object with `diagnostics` and `statistics`;
SARIF stores statistics in run properties.
Statistics include rule counts and suppression reasons and states. GitHub
statistics go to stderr so stdout contains only annotation commands.

| Exit code | Meaning |
| --- | --- |
| `0` | The check completed without violations, or `--exit-zero` was used. |
| `1` | The check completed and found violations. |
| `2` | A tool error or incomplete check occurred; any collected diagnostics are still reported. |

`--exit-zero` preserves exit code `2` for errors. Unchecked paths, inactive
preview selectors, and a check with no enabled rules print a notice to stderr
and keep exit code `0`; inspect the policy before using such a check as a gate.

## Explain an exception

Place a suppression before the relevant block and give a reviewable reason:

```markdown
<!-- seiso: allow LNK001 -- The site generator creates this page. -->
[Generated API](generated/api.md)
```

Use complete rule codes. `seiso rule SUP001` explains accepted syntax and
`seiso rule SUP002` explains how unused suppressions are reported.

## Apply safe fixes

```sh
seiso check --fix
```

The safe fixer removes confirmed unused suppression codes and checks the
result again. It preserves codes that are active, disabled, invalid, or
undetermined, including preview rules that were not enabled. A partially
stale declaration keeps its remaining codes and reason. No fixes are applied
after an incomplete check, and stdin checks cannot use `--fix`. With no safe
edits, it returns the initial result. A real edit plan is revalidated against
fresh inputs before writing; changed sources prevent the planned write.

## Inspect cached results

Checks store content-derived parse data in `.seiso_cache/`. Each run reads and
hashes its required sources, then resolves path policy and links against the current
workspace. Moving a file, editing configuration, or deleting a link target
takes effect even when source content was cached. The directory contains its
own `.gitignore`, and seiso removes expired entries itself; deleting the
directory is always safe.

Use `--no-cache` to bypass cache reads and writes. Missing, corrupt, outdated,
or unwritable cache entries fall back to parsing; diagnostic output is the
same with a cold, warm, or disabled cache.

For editor and submission gates, follow [Integrate checks](integrations.md).
