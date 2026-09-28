<div align="center">

<img src="https://github.com/scarletkc/seiso/raw/main/assets/logo.svg" alt="seiso logo" width="128" />

# seiso

**A Markdown convention and linter for project docs written by AI and read by humans and agents.**

[![CI](https://img.shields.io/github/actions/workflow/status/scarletkc/seiso/ci.yml?branch=main&label=CI&logo=githubactions&logoColor=white)](https://github.com/scarletkc/seiso/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/seiso?logo=rust&label=crates.io)](https://crates.io/crates/seiso)
[![PyPI](https://img.shields.io/pypi/v/seiso?logo=pypi&logoColor=white&label=PyPI)](https://pypi.org/project/seiso/)
[![npm](https://img.shields.io/npm/v/%40scarletkc%2Fseiso?logo=npm&label=npm)](https://www.npmjs.com/package/@scarletkc/seiso)
[![License](https://img.shields.io/badge/license-MIT-blue)](https://github.com/scarletkc/seiso/blob/main/LICENSE)
[![CodeRabbit Reviews](https://img.shields.io/coderabbit/prs/github/scarletkc/seiso?label=CodeRabbit%20Reviews&labelColor=171717&color=FF570A)](https://coderabbit.ai)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/scarletkc/seiso)

</div>

Hardly anyone writes project docs by hand anymore. AI writes most of them,
and coding agents read them as context as often as people do. Both kinds of
reader take the page at its word, so a stale version number or a field list
updated in only one of its three copies misleads all of them. AI writes docs
faster than anyone can review them, and it makes the same few mistakes every
time.

seiso defines one convention for how Markdown in a repository is organized,
so projects can follow the same rules without agreeing on them first, the
way rustfmt settled formatting for Rust code.

## The convention

- Every document declares one kind, such as `howto`, `reference`, or `adr`,
  and holds only what that kind is for. A how-to gives the steps. Why the
  design looks this way belongs in an ADR.
- Each fact has one home. Other pages link to it instead of retelling it.
- Long-lived pages don't record values that change faster than the page,
  such as versions, deployment status, or counts.
- A pointer names a file or symbol, so the reader doesn't have to search for
  what the sentence promised.
- The finished page doesn't address whoever asked for it or narrate how it
  was made.
- Judgment calls a tool can't make are written down with a reason. An
  exception without one is itself a violation.

seiso's rules check documents against this convention; stable rules run by
default, and the rest are opt-in previews. Each diagnostic says where
the problem is and how to fix it, so an agent can repair the page from
seiso's output alone. seiso doesn't guess whether prose sounds
machine-written, and it leaves formatting and spelling to other tools. The
[convention](https://github.com/scarletkc/seiso/blob/main/docs/reference/convention.md)
defines each kind's contract and the evidence a diagnostic can claim.

## Install

```sh
cargo install seiso
uv tool install seiso    # or: pipx install seiso
npm install -g @scarletkc/seiso
```

The PyPI package includes prebuilt binaries for Linux x64 and Windows x64 and
builds from source on other platforms, which requires a Rust toolchain. The npm
package installs only on Linux x64 with glibc and on Windows x64; elsewhere, use
cargo or the PyPI package. From a source checkout, run
`cargo run -p seiso -- <command>`.

## Quick start

From the repository root:

```sh
seiso init
seiso check
```

`seiso init` writes a `seiso.toml` at the repository root with suggested
exclusions and kind mappings; review them before relying on the results.

`seiso check` runs only stable rules, which have met the
[promotion criteria](https://github.com/scarletkc/seiso/blob/main/docs/evaluation/policy.md).
The other rules are in preview: they are experimental, can report false
positives, and run only with `--preview`. Try them locally before relying on
them, and keep them out of CI gates. `seiso rule --all` lists every rule with
its status, `seiso rule <CODE>` explains a rule with examples, and `seiso parse`
inspects the document model without running rules.

## Documentation

- [Checking documents](https://github.com/scarletkc/seiso/blob/main/docs/guides/checking.md): configuration, rule selection, and output formats
- [Integrations](https://github.com/scarletkc/seiso/blob/main/docs/guides/integrations.md): Claude Code hooks, pre-commit, and CI
- [Development](https://github.com/scarletkc/seiso/blob/main/docs/guides/development.md): building, testing, and validation commands
- [Architecture](https://github.com/scarletkc/seiso/blob/main/docs/reference/architecture.md): command execution
- [Roadmap](https://github.com/scarletkc/seiso/blob/main/docs/design/roadmap.md): proposed features and milestone evidence
- [Documentation index](https://github.com/scarletkc/seiso/blob/main/docs/README.md): guides, references, and evaluation records
- [Contributing](https://github.com/scarletkc/seiso/blob/main/CONTRIBUTING.md): issues, branches, commits, and pull requests

## License

seiso is licensed under [MIT](https://github.com/scarletkc/seiso/blob/main/LICENSE).
