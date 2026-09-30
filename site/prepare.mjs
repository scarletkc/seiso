// Writes the site's sources into src/: one directory per released
// specification version, read from its spec-v<version> tag so that the
// repository holds no copies, redirects to the newest version, and the
// repository's logo.
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const site = dirname(fileURLToPath(import.meta.url))
const source = join(site, 'src')
const ORDER = ['convention', 'examples', 'adopting', 'CHANGELOG']

export const UNRELEASED = 'unreleased'

// From a subdirectory, ls-tree would list only that part of a tree.
function git(...args) {
  return execFileSync('git', args, { cwd: join(site, '..'), encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
}

function releasedVersions() {
  return git('tag', '--list', 'spec-v*')
    .split('\n')
    .map((tag) => /^spec-v(\d+)\.(\d+)\.(\d+)$/.exec(tag.trim()))
    .filter(Boolean)
    .sort((a, b) => b[1] - a[1] || b[2] - a[2] || b[3] - a[3])
    .map((match) => match[0].slice('spec-v'.length))
}

function releasedFiles(version) {
  const tag = `spec-v${version}`
  return git('ls-tree', '--name-only', `${tag}:spec`)
    .split('\n')
    .filter((name) => name.endsWith('.md'))
    .map((name) => [name, git('show', `${tag}:spec/${name}`)])
}

function worktreeFiles() {
  const spec = join(site, '..', 'spec')
  return readdirSync(spec)
    .filter((name) => name.endsWith('.md'))
    .map((name) => [name, readFileSync(join(spec, name), 'utf8')])
}

function title(markdown, fallback) {
  return /^# (.+)$/m.exec(markdown)?.[1].trim() ?? fallback
}

// Pages that ORDER does not name sort before the changelog.
function rank(name) {
  const index = ORDER.indexOf(name)
  return index === -1 ? ORDER.indexOf('CHANGELOG') - 0.5 : index
}

function redirect(target) {
  return `---
layout: false
head:
  - - meta
    - http-equiv: refresh
      content: 0; url=${target}
---

<script setup>
if (typeof window !== 'undefined') window.location.replace('${target}' + window.location.hash)
</script>

[${target}](${target})
`
}

// Returns the versions newest first, each with its pages in sidebar order.
// With `unreleased`, the working tree's spec/ is added after them, for
// checking that unreleased text builds; it is never deployed.
export function prepare({ unreleased = false } = {}) {
  const versions = releasedVersions().map((version) => ({ version, files: releasedFiles(version) }))
  if (versions.length === 0) {
    throw new Error('No spec-v<version> tag found; fetch tags before building the site')
  }
  if (unreleased) versions.push({ version: UNRELEASED, files: worktreeFiles() })

  rmSync(source, { recursive: true, force: true })
  mkdirSync(join(source, 'public'), { recursive: true })
  copyFileSync(join(site, '..', 'assets', 'logo.svg'), join(source, 'public', 'logo.svg'))
  const result = versions.map(({ version, files }) => {
    mkdirSync(join(source, version), { recursive: true })
    const pages = files.map(([file, markdown]) => {
      writeFileSync(join(source, version, file), markdown)
      const name = file.slice(0, -'.md'.length)
      return { name, title: title(markdown, name) }
    })
    pages.sort((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name))
    return { version, released: version !== UNRELEASED, pages }
  })

  // A directory path such as /0.1.0/ opens that version's specification.
  for (const { version } of result) {
    writeFileSync(join(source, version, 'index.md'), redirect(`/${version}/convention`))
  }
  const newest = result[0]
  writeFileSync(join(source, 'index.md'), redirect(`/${newest.version}/convention`))
  mkdirSync(join(source, 'latest'))
  writeFileSync(join(source, 'latest', 'index.md'), redirect(`/${newest.version}/convention`))
  for (const { name } of newest.pages) {
    writeFileSync(join(source, 'latest', `${name}.md`), redirect(`/${newest.version}/${name}`))
  }
  return result
}
