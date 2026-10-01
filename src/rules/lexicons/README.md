---
kind: reference
---

# Built-in phrase lexicons

[normative.toml](normative.toml) and [heuristic.toml](heuristic.toml) contain
language-specific trigger and constraint entries used by the convention
and heuristic sentence rules.
Generic syntactic checks, numeric patterns, section classification, and
evidence detection remain in their owning modules.

Each `[[group.language]]` entry declares a `phrase`, optional `guard`, and
nonempty `hits` and `misses` arrays. Languages are `en`, `zh`, and `ja`.
Examples contain `text` (Markdown) and an optional `target` for rules that
report a value or link instead of the phrase. Examples run as `howto`
documents with an explicit language, preventing language detection from
silently testing a different lexicon.

| Group | Owning rule | Diagnostic location |
| --- | --- | --- |
| stale | STL001 | volatile value, named by `target` |
| constraint | STL001, also used by STL004 | suppresses a value diagnostic |
| commit | STL003 | hash, named by `target` |
| pointer | PTR001 | repository link, named by `target` |
| source | PTR003 | phrase |
| rationale | RAT002 | heading phrase |
| conversation | VOX001 | phrase |
| deployment | STL002 | phrase |
| excluded_heading | VOX002 | heading phrase |
| production_heading, narration | VOX003 | phrase |
| evaluation | EVD001 | phrase |

`guard = "end-user"` rejects negation, condition, requirement, or verification
context in the phrase's own clause, and attributive or temporal suffixes.
It is optional for any entry. Conversation entries and configured
conversation additions use this guard; other configured additions are
unguarded. Matching uses ASCII case folding and word boundaries for English,
character matching for CJK, and source span mappings.

Ordinary `hits` require a diagnostic from the owning rule at the tested
phrase or explicit `target`; `misses` require no diagnostic. Each example
must pass with the full lexicon and with its entry isolated from siblings.
Constraint entries are suppression cues: their hits suppress a snapshot's
value diagnostic, while their misses put the constraint in another sentence
and require that diagnostic. The test
`every_builtin_phrase_has_executable_intended_use_examples` in
[lexicon.rs](../lexicon.rs) implements this contract.

These examples are regression/tuning fixtures; the
[evaluation policy](../../../docs/evaluation/policy.md) defines evidence for
accuracy claims and rule promotion.
