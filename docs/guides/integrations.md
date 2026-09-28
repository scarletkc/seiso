---
kind: howto
---

# Integrate checks

Configure and run [a local check](checking.md) before adding an automated gate.
The integrations use stable rules by default. Preview rules are experimental
and can report false positives, so try them with a local
`seiso check --preview` before enabling them in a hook or CI gate.

## Claude Code

Add a command hook in `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          { "type": "command", "command": "seiso hook claude-code" }
        ]
      }
    ]
  }
}
```

The adapter reads the event's `cwd` and `tool_input.file_path`. It checks the
saved Markdown file and sends concise diagnostics to stderr. Non-Markdown
paths return success without output.

When cross-file rules are enabled, the workspace index also allows feedback
from documents that refer to the edited file. Parse caching and
[input scope](checking.md#select-files-and-rules) are shared with ordinary checks.

Violations return hook exit code `2`, which sends feedback to Claude. Tool errors
and incomplete checks return `1` for non-blocking error reporting. A successful
check returns `0` without output. These mappings follow the
[Claude Code hook protocol](https://code.claude.com/docs/en/hooks#exit-code-output).

PostToolUse runs after the edit and cannot undo it. The Write/Edit matcher does
not cover files written by shell commands or other processes; run checks again
before submission.

## pre-commit

Add the hook to `.pre-commit-config.yaml` and set `rev` to a seiso
[release tag](https://github.com/scarletkc/seiso/releases);
`pre-commit autoupdate` later moves it to the newest release:

```yaml
repos:
  - repo: https://github.com/scarletkc/seiso
    rev: <release-tag>
    hooks:
      - id: seiso
```

The hook selects Markdown files and runs `seiso check` on the changed paths.
Its Python environment builds the CLI through maturin, so installation requires
the Rust toolchain. Run `pre-commit run seiso --all-files` to check the workspace.

## CI

Install a reviewed build and run:

```sh
seiso check --output-format concise
```

Use the [exit codes](checking.md#consume-results) as the gate. To review changes
to exclusions, kind mappings, rule selection, and suppression reasons, save
`seiso policy` from both revisions and compare the JSON. Review policy
changes alongside document edits.
Use `seiso policy --evaluate` when the review also needs current suppression
outcomes; ordinary policy inspection leaves enabled codes unevaluated.

For GitHub Actions annotations, use `seiso check --output-format github`.
For tools that consume SARIF, save `seiso check --output-format sarif` to a
file and upload it through that tool's integration. Diagnostic output does
not replace the command's exit status.
