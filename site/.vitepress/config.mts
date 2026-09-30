import { posix } from 'node:path'
import { defineConfig, type DefaultTheme, type MarkdownOptions } from 'vitepress'
import { prepare, UNRELEASED } from '../prepare.mjs'

const REPOSITORY = 'https://github.com/scarletkc/seiso'
const versions = prepare({ unreleased: process.env.SITE_UNRELEASED === '1' })
const newest = versions[0].version

type Markdown = Parameters<NonNullable<MarkdownOptions['config']>>[0]

function pageVersion(env: { relativePath?: string }) {
  const [version, page] = String(env.relativePath ?? '').split('/')
  return versions.some((entry) => entry.version === version) ? { version, page } : undefined
}

// Links that leave spec/ open the repository at the same release; the
// unreleased text links to main.
function repositoryLinks(md: Markdown) {
  md.core.ruler.push('repository_links', (state) => {
    const current = pageVersion(state.env)
    if (!current) return
    const ref = current.version === UNRELEASED ? 'main' : `spec-v${current.version}`
    for (const token of state.tokens) {
      for (const child of token.children ?? []) {
        const href = child.type === 'link_open' ? (child.attrGet('href') ?? '') : ''
        if (!href || /^([a-z][a-z0-9+.-]*:|[#/])/i.test(href)) continue
        const cut = href.search(/[?#]/)
        const path = cut === -1 ? href : href.slice(0, cut)
        const target = posix.normalize(posix.join('spec', path))
        if (!target.startsWith('spec/')) {
          child.attrSet('href', `${REPOSITORY}/blob/${ref}/${target}${href.slice(path.length)}`)
        }
      }
    }
  })
}

// On the specification page, a paragraph that opens with a requirement
// identifier such as `KIND-1` takes it as its anchor, and the identifier
// links there.
function requirementAnchors(md: Markdown) {
  md.core.ruler.push('requirement_anchors', (state) => {
    if (pageVersion(state.env)?.page !== 'convention.md') return
    state.tokens.forEach((token, index) => {
      const inline = state.tokens[index + 1]
      const first = inline?.children?.[0]
      if (token.type !== 'paragraph_open' || first?.type !== 'code_inline') return
      if (!/^[A-Z]+-\d+$/.test(first.content)) return
      token.attrSet('id', first.content)
      const open = new state.Token('link_open', 'a', 1)
      open.attrSet('href', `#${first.content}`)
      open.attrSet('class', 'requirement')
      inline.children!.splice(0, 1, open, first, new state.Token('link_close', 'a', -1))
    })
  })
}

export default defineConfig<DefaultTheme.Config & { versions: typeof versions }>({
  lang: 'en-US',
  title: 'Seiso Convention',
  description: 'A specification for the Markdown documents of software projects',
  srcDir: 'src',
  cleanUrls: true,
  head: [['link', { rel: 'icon', type: 'image/svg+xml', href: '/logo.svg' }]],
  markdown: {
    // The sources are GitHub Markdown; `{...}` stays text.
    attrs: { disable: true },
    config(md) {
      md.use(repositoryLinks).use(requirementAnchors)
    },
  },
  themeConfig: {
    logo: { src: '/logo.svg', alt: '' },
    logoLink: `/${newest}/convention`,
    nav: [
      {
        text: 'Versions',
        items: versions.map(({ version }) => ({ text: version, link: `/${version}/convention` })),
      },
    ],
    sidebar: Object.fromEntries(
      versions.map(({ version, pages }) => [
        `/${version}/`,
        pages.map(({ name, title }) => ({ text: title, link: `/${version}/${name}` })),
      ]),
    ),
    socialLinks: [{ icon: 'github', link: REPOSITORY }],
    versions,
  },
})
