# Contributing to seiso

Changes to seiso start as GitHub issues and land through pull requests. This
guide covers how to propose a change, name a branch, write commits, and open a
pull request. The [development guide](docs/guides/development.md) covers
building and testing.

## Start with an issue

Open or find an issue before writing code. A pull request without an issue is
acceptable only for a small, self-evident fix, such as a typo or a broken link.
For anything larger, agree on the direction with a maintainer in the issue
before starting the work.

Search existing issues first. Title the issue with a one-line summary of the
problem rather than the solution, for example
`LNK002 misses anchors defined by HTML id attributes`. Labels such as `bug`,
`enhancement`, and `documentation` classify the issue, so the title needs no
type prefix.

### Describe the problem first

The reason for a change decides whether it belongs in seiso, so every issue
opens with it. For a feature or rule request, describe:

- the documentation problem or workflow that fails today, with a real example;
- who runs into it: human readers, coding agents, maintainers, or CI;
- what happens if nothing changes, and any workaround in use.

Put a proposed solution after the problem, if you have one. A request that
describes only a solution is sent back for its motivation before it is
considered.

### Include what the issue needs

- **Bug**: the output of `seiso --version`, the command, a minimal document and
  configuration that reproduce the problem, the full output, and the expected
  result.
- **Incorrect diagnostic**: the rule code, the smallest excerpt that triggers
  or misses it, and why the result is wrong.
- **New rule or convention change**: real documents that show the problem. New
  rules start in preview and must meet the
  [evaluation policy](docs/evaluation/policy.md) before they become stable.

## Name the branch

Branch names follow [Conventional Branch](https://conventionalbranch.org/):
`<type>/issue-<number>-<description>`, for example
`fix/issue-42-html-id-anchors`. Leave out the `issue-<number>-` part only for
a change without an issue.

| Type | Use for |
| --- | --- |
| `feat/` | New features, rules, options, or output formats |
| `fix/` | Bug fixes and incorrect diagnostics |
| `hotfix/` | Urgent fixes to a published release |
| `release/` | Release preparation by maintainers, for example `release/v0.1.0` |
| `chore/` | Documentation, tests, CI, build, refactoring, and dependency updates |

Use lowercase letters, digits, and single hyphens. Dots are allowed only in
release versions.

Branch from the latest `main` and keep each branch to one change.

## Write commits

Commit messages follow
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<scope>): <description>

<body>

<footer>
```

| Type | Use for |
| --- | --- |
| `feat` | A new feature, rule, option, or output format |
| `fix` | A bug fix, including an incorrect diagnostic |
| `perf` | A performance improvement without a behavior change |
| `refactor` | A code change that neither fixes a bug nor adds a feature |
| `docs` | Documentation only |
| `test` | Tests only |
| `build` | Packaging, distribution, or dependencies |
| `ci` | GitHub Actions workflows and CI scripts |
| `style` | Formatting only |
| `chore` | Maintenance that fits no other type |
| `revert` | Reverting an earlier commit |

- The scope is optional. Use a lowercase module or area name, such as `rules`,
  `config`, `cache`, `release`, or `evaluation`.
- Write the description in the imperative mood, without a trailing period,
  for example `fix(rules): report anchors from HTML id attributes`. Start it
  with a lowercase word; identifiers such as rule codes keep their case. Keep
  the header within 72 characters.
- Use the body to explain what changed and why, wrapped at 72 characters.
- Mark a breaking change with `!` after the type or scope and a
  `BREAKING CHANGE:` footer. Changes to CLI options, configuration keys, exit
  codes, rule codes, and JSON or SARIF output are breaking.
- Reference issues in the footer, for example `Refs #42`.

## Open a pull request

Title the pull request with a Conventional Commits header, as for a commit.
Pull requests are squash-merged, and the title becomes the commit on `main`.
The [PR title workflow](.github/workflows/pr-title.yml) checks the title and
runs again when you edit it.

The description gives a reviewer the context that the diff cannot:

- **Context**: the problem this solves and why it matters now, with a link to
  the issue. Summarize any decisions reached in the issue discussion.
- **Changes**: what changed, and the choices a reviewer might question.
- **Validation**: the commands you ran and their results.
- **Breaking changes**: what users must change, if anything.

Link the issue with a closing keyword such as `Closes #42`, or with `Refs #42`
when the pull request addresses only part of it.

Before requesting review:

- Run the checks in the
  [development guide](docs/guides/development.md#build-and-validate). CI runs
  on every update to the pull request, and its `check` job must pass.
- When behavior changes, update the documentation, command help, and rule
  explanations in `docs/rules/` in the same pull request. Markdown in this
  repository must pass `seiso check`.
- Update the branch with the latest `main`; only an up-to-date branch can merge.
- Leave version numbers and release notes unchanged. Maintainers prepare them
  when [publishing](docs/guides/publishing.md) a release.

Open the pull request as a draft while the work is in progress. Merging
requires approval from a maintainer.

## License

Contributions are licensed under the project's [MIT License](LICENSE).
