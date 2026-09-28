---
kind: reference
---

# Section classification

`seiso parse --output-format json` exposes `section_annotations` beside each
file's `document`. The library entry point is `sections::classify`. These
heuristic predictions describe section responsibilities; document `kind`
continues to come from frontmatter or configuration.

## Annotation fields

| Field | Meaning |
| --- | --- |
| `section` | Index into `document.sections`, including the headingless root |
| `section_type` | Predicted responsibility from the table below |
| `content_span` | Original byte range after the heading and before the first child section |
| `evidence` | Signal names and original source spans supporting the prediction |

| Type | Signals |
| --- | --- |
| `steps` | Procedure heading, actionable prose, or a command/configuration example |
| `reference` | API or reference heading, table, field, or return contract |
| `rationale` | Decision heading or explicit choice/trade-off prose |
| `background` | Background, overview, or concepts heading |
| `troubleshooting` | Troubleshooting, FAQ, exceptions, or known-issues heading |
| `other` | No recognized signal |

## Classification boundaries

Explicit recovery and decision headings, API headings, and reference content
supply specific role evidence. Generic heading words are matched as whole
labels rather than substrings in unrelated titles. Reference content includes
tables, return contracts, and field requirements. Actionable prose and code
introduced by instructions can establish steps. Extended explanatory prose
supplies background when stronger evidence is absent; quizzes and navigation
lists remain unclassified. The signal names and phrase lists are defined in
[`sections`](../../src/sections.rs).

Only direct section blocks contribute; parent sections do not inherit child
roles. Quoted phrases are masked by the shared prose-assertion reader.
Quotations, footnotes, sample output, and console output described as
an error/result do not establish a main flow. A console fence needs a command
prompt. A non-shell example needs an adjacent instruction. The classifier does
not execute code or interpret opaque site components. It assigns no probability;
`other` includes unsupported and missed responsibilities.

Ordering rules remain incomplete when no main flow is detected, or when opaque
HTML/component content or an unclassified code example could hide an earlier
procedure. That state prevents SUP002 from removing a suppression
whose rule could not establish an outcome.

Classification runs independently of document kind, preview selection, and
`lint.languages`, so inspection can expose missed cases. Rules apply their own
kind scope and language selection afterward. Sentence rules use sentence
language; structural rules use the document language, with sentence language
also checked for rationale evidence.

These predictions are recomputed from parsed content. They do not alter the
parse cache or grant a generated-document exemption. The
[evaluation procedure](../../corpus/docs/evaluation.md#heuristic-evaluation)
separates predicted annotations from reviewed labels.
