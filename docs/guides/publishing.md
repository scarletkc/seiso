---
kind: howto
---

# Publish seiso

Use [Build and publish distributions](../../.github/workflows/publish.yml) to
publish `seiso` on crates.io, `@scarletkc/seiso` on npm, `seiso` on PyPI, and
a GitHub Release from one release commit. All three package installations
provide the `seiso` executable. The workflow runs only through
`workflow_dispatch`; all upload inputs default to false.

## Prepare the version and release notes

From the repository root, run:

```sh
python -m scripts.release.bump_version patch --note "Release title"
```

The version argument accepts `patch` (also the default), `minor`, `major`,
or an explicit version such as `1.2.3`, `v1.2.3`, or `1.2.3-rc.1`. Add `--dry-run` to preview
the affected files. The script rejects equal or lower versions, checks the
existing versions for consistency, and plans all edits before writing.

The script updates `package.version` in the root `Cargo.toml`, the `seiso`
entry in `Cargo.lock`, and the version and platform package pins in
`npm/seiso/package.json`. Third-party dependency versions remain unchanged.
clap reads the package version for the CLI's `--version`, and maturin reads
it for PyPI. No separate CLI or Python version
literal needs editing. Release versions use `MAJOR.MINOR.PATCH`, optionally
followed by `-alpha.N`, `-beta.N`, or `-rc.N`. Python distribution metadata uses
the corresponding PEP 440 spelling; Cargo, npm, Git tags, release-note filenames,
and the executable retain the Cargo spelling. Other suffixes,
including build metadata, are outside this shared registry version format.
From a prerelease, `patch` promotes its version to the final release; use an
explicit version to advance the prerelease number.

| Cargo / npm / CLI | Python metadata | npm dist-tag | GitHub Release |
| --- | --- | --- | --- |
| `1.2.3-alpha.1` | `1.2.3a1` | `alpha` | Pre-release, not latest |
| `1.2.3-beta.1` | `1.2.3b1` | `beta` | Pre-release, not latest |
| `1.2.3-rc.1` | `1.2.3rc1` | `rc` | Pre-release, not latest |
| `1.2.3` | `1.2.3` | `latest` | Stable release |

Version ordering is semantic: `alpha.9 < alpha.10 < beta.1 < rc.1 < stable`
for the same base version. Prereleases leave npm's `latest` tag and GitHub's
latest stable release unchanged. Parsing, ordering, and Python normalization
are shared in [`scripts/release/versions.py`](../../scripts/release/versions.py).

For example, start a prerelease cycle with:

```sh
python -m scripts.release.bump_version v1.2.3-alpha.1 --note "Alpha preview"
```

After each release, advance explicitly to `1.2.3-alpha.2`, `1.2.3-beta.1`,
or `1.2.3-rc.1` using the same command. Each bump requires a version greater
than the current Cargo version.

`--note` creates `docs/release-notes/VERSION.md` with a `## Release title`
heading. Fill in its body with user-facing changes and migration instructions
before committing. Existing notes are never overwritten. The note is optional,
but a supplied file with an invalid heading or no body fails preflight.

CI appends an automatic `Changelog` to the handwritten note, with a full diff
link when a comparison base exists. Only canonical `vVERSION` tags reachable
from the release commit and semantically lower than the target qualify:

- A prerelease uses the highest qualifying version, including prereleases.
  The first alpha can compare to the preceding stable release; a later alpha,
  beta, or rc compares to the preceding release in that cycle.
- A stable release uses the highest qualifying **stable** version, so its
  changelog includes changes made throughout the prerelease cycle.
- Without a qualifying base, the changelog lists the full reachable commit
  history. The target tag is excluded, so retries keep the same comparison.

For example, `1.2.3-alpha.1` can compare to `v1.2.2`, `1.2.3-alpha.10` to
`v1.2.3-alpha.9`, and `1.2.3` to `v1.2.2` even when `v1.2.3-rc.1` exists.
Tag annotation and creation order do not affect selection. Preview the body:

```sh
python -m scripts.release.github_release notes --output target/release-notes.md
```

An existing `vVERSION` tag must point to the selected release commit; a
conflicting tag fails preflight before any upload. Commit the version changes
and completed note together, then validate that release ref.

## Validate and build without publishing

Use Python 3.12 or later and current stable Rust:

```sh
python -m scripts.release.release check
python -m unittest discover -s scripts/tests -p 'test_*.py'
cargo test --locked
python -m scripts.release.release crates
```

The last command runs `cargo package --package seiso --locked --registry
crates-io`, including compilation of the packaged sources without uploading. The package
contains the CLI, library modules, embedded rule documentation, and MIT license.
Release validation identifies the root `seiso` package; other workspace packages
may have their own versions and publication settings. Cargo's package verification
checks dependency publishability and compilation, including versioned local dependencies.
The root package's crates.io publication setting is checked only on the crates
path; it does not block Python, npm, or GitHub distribution.
Already published versions are immutable; bump the version before releasing
changed code.

In GitHub Actions, select **Build and publish distributions → Run workflow**
and choose the release branch or tag. Leave all `publish_*` checkboxes unchecked.

The workflow checks version consistency and release notes, runs script and
Rust tests, verifies the Cargo package, builds and exercises a wheel on each
platform in the `wheels` job matrix, creates a source archive, packs the npm
packages, and exercises the npm executable on each of those platforms. musl
wheels and npm installations are exercised in Alpine containers. npm
installations resolve `@scarletkc/seiso` from a local registry that serves the
packed archives
([`npm_registry.py`](../../scripts/release/npm_registry.py)), so npm selects
the platform package as it would from npmjs.com. Installation checks require
CLI output to match Cargo's SemVer and installed Python metadata to match its
PEP 440 version. Build jobs have no publishing secrets or OIDC permissions and
do not enter publishing environments. This is the complete
validation path. A selected registry upload waits for shared preflight and its
required artifacts:

| Selection | Builds and verifies |
| --- | --- |
| `publish_crates` | Cargo package |
| `publish_pypi` | Wheels and source archive |
| `publish_npm` | Wheels and source archive, then the npm packages using those binaries |
| `publish_github`, `publish_all`, or no upload selection | All distributions and release notes |

Selections are additive. Script tests and Linux x64 Rust tests run once in
shared preflight; the other wheel jobs, except the musl ones, also run the
Rust tests on their own platforms. Release notes and tag checks apply to the
full validation and GitHub Release paths.

Download the artifacts and generated release body:

```sh
gh run download RUN_ID -p 'distributions-*' -p npm-package -p cargo-package -p release-notes -D dist/downloaded
```

`@scarletkc/seiso` requires Node.js 18 or later. It contains the `seiso`
launcher and pins one `@scarletkc/seiso-PLATFORM` package per entry in
`NPM_PLATFORMS` in [`prepare_npm.py`](../../scripts/release/prepare_npm.py) as
an optional dependency. Each platform package holds the executable from the
matching verified wheel and declares `os`, `cpu`, and on Linux `libc`, so npm
installs only the package that matches. The launcher picks the musl package
when Node reports no glibc. Nothing downloads or builds at install time. Source
installations require Rust.

## Authentication

The workflow uses these GitHub environments and credentials:

| Destination | Environment | Authentication |
| --- | --- | --- |
| npm | `npm` | Trusted Publishing (OIDC), with direct `npm publish` allowed |
| crates.io | `crates-io` | Trusted Publishing (OIDC) for `seiso` |
| PyPI | `pypi` | Trusted Publishing (OIDC) |
| GitHub Release | `github-release` | Built-in `GITHUB_TOKEN` with `contents: write` |

Trusted Publisher configurations must match the repository, `publish.yml`,
and the environment. Registry upload jobs alone receive `id-token: write`.
Build jobs require no registry credentials. Environment deployment rules must
permit the selected release ref.

On npm, `@scarletkc/seiso` and every platform package need their own Trusted
Publisher configuration, and npm accepts one only for a package that already
exists. When `NPM_PLATFORMS` gains a platform, publish a `0.0.0` placeholder of
its package by hand: a directory with only a `package.json` that sets the name
and version, and a README saying that the version has no executable. Then
configure its Trusted Publisher with npm 11.15.0 or later:

```sh
npm publish PLACEHOLDER_DIRECTORY --access public
npm trust github @scarletkc/seiso-PLATFORM --file publish.yml --repo scarletkc/seiso --env npm --allow-publish
```

`npm trust` requires two-factor authentication on the account; the package's
settings page on npmjs.com offers the same configuration. The workflow
publishes every real version.

crates.io authentication uses a temporary token from
`rust-lang/crates-io-auth-action`. No stored registry publishing token is needed.

## Publish and recover a partial release

Run the workflow at the tested release ref and enable **`publish_all`** to
publish npm, crates.io, PyPI, and a GitHub Release in one run. For selected
registries, leave it off and enable `publish_npm`, `publish_crates`, or
`publish_pypi`. Select `publish_github` as well to create the GitHub Release
after the selected uploads succeed. On its own, `publish_github` builds all
distributions and creates the GitHub Release without registry uploads.

The GitHub Release is named `seiso vVERSION`, tags the checked-out commit, and
includes the generated body, wheels, source archive, npm archives, and the
`seiso` crate archive. Registry uploads are independent after shared validation; a failure
in one cannot roll back another. GitHub Release creation waits for all selected
registries to succeed.

Prereleases use the same workflow and controls. For example, after committing
and pushing the prerelease bump on `release/v1.2.3-alpha.1`, validate and then
publish that unchanged ref:

```sh
gh workflow run publish.yml --ref release/v1.2.3-alpha.1
# After the validation run succeeds:
gh workflow run publish.yml --ref release/v1.2.3-alpha.1 -f publish_all=true
```

Re-run the same release commit to finish a partial release, or select only the
failed destination:

- npm checks each package's exact version and skips those that exist. Platform
  packages upload before `@scarletkc/seiso`, so the main package never pins a
  missing platform package. New versions use the tested archives without
  repacking them.
- crates.io checks the exact `seiso` version in its sparse index and skips an
  existing version. Yanked versions stop the release.
- PyPI retains `skip-existing: true` and uploads missing distribution files.
- An existing GitHub Release keeps its body and assets; retrying uploads only
  missing attachments. A draft release or a prerelease flag inconsistent with
  the version stops the retry for manual correction.

Only HTTP 404 means an absent registry resource. Network, authorization,
rate-limit, malformed-response, and publication errors fail the job. If Cargo
times out after uploading, check crates.io before retrying: the upload may have
succeeded. Skipping a duplicate confirms the version exists; it does not prove
its contents match changed local source.

Verify each registry exposes the intended version, check the GitHub tag's
commit and attachments, then install the exact version in a clean environment
and run `seiso --version` and `seiso parse`. Registry publication alone does not
establish the milestone's corpus acceptance criteria.

## Install a prerelease and promote to stable

Choose one distribution to try an exact release:

```sh
cargo install seiso --version '=1.2.3-rc.1' --locked
npm install --global @scarletkc/seiso@1.2.3-rc.1
python -m pip install 'seiso==1.2.3rc1'
```

For the most recent release in an npm channel, use `@scarletkc/seiso@alpha`,
`@scarletkc/seiso@beta`, or `@scarletkc/seiso@rc`. For Python, `python -m pip
install --upgrade --pre seiso` allows prereleases. Verify the installed CLI with
`seiso --version`; even the Python installation reports `seiso 1.2.3-rc.1`.

From `1.2.3-rc.1`, promote to `1.2.3` with:

```sh
python -m scripts.release.bump_version patch --note "Stable release"
```

Fill in `docs/release-notes/1.2.3.md`, commit the bump, and repeat validation and
publication for that commit. Promotion builds new stable artifacts; it does
not rename prerelease artifacts. The stable npm upload updates `latest`, and
GitHub creates a stable release. npm channel tags continue to identify their
last prereleases. Subsequent `patch` bumps advance the patch number normally.
