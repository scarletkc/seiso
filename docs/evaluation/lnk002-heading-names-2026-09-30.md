---
kind: adr
---

# LNK002 heading name decision

Recorded on 2026-09-30 for [issue 11](https://github.com/scarletkc/seiso/issues/11).
It decides how LNK002 treats heading `name` attributes before the fresh
holdout freezes the implementation. It supplies no holdout evidence and does
not promote LNK002.

## Evidence

The source is `BurntSushi/ripgrep` at
`3fce3b5bb0236da2df6d99672afb8a719642eca7`. The
[exact configuration heading](../../corpus/results/lnk002/name-attribute/config-heading.txt)
declares `name="config"` on an `h3`. The
[Markdown API request](../../corpus/results/lnk002/name-attribute/markdown-request.json)
contains all 27 original heading blocks. Its successful
[GFM response](../../corpus/results/lnk002/name-attribute/markdown-api.html)
retains the heading names with GitHub's `user-content-` prefix.

The [browser measurements](../../corpus/results/lnk002/name-attribute/browser.json)
use Chromium 151, a 1280 by 800 viewport, and fresh pages for each control.
GitHub's CSS and JavaScript loaded with no failed resource requests. TLS
certificate verification remained enabled.

| Fragment | Scroll Y | Configuration heading viewport Y |
| --- | ---: | ---: |
| No fragment | 0 | 1220.359375 |
| `config` | 833 | 387.359375 |
| `does-ripgrep-support-configuration-files` | 1094 | 126.359375 |
| Nonexistent control | 0 | 1220.359375 |

The heading's document position is 1220.359375 in every case. The ordinary
slug control works, and `#config` brings the name-only heading into view.
The [screenshot](../../corpus/results/lnk002/name-attribute/name-config.png)
shows the `#config` case. To reproduce the check, run
`check_github.py --output NEW_DIRECTORY` with Playwright and its Chromium
installed.

The [HTML Standard](https://html.spec.whatwg.org/multipage/browsing-the-web.html#the-indicated-part-of-the-document)
selects an element's `id`, then a matching `name` on an `a` element. The
[standard receipt](../../corpus/results/lnk002/name-attribute/html-standard.json)
records that native HTML navigation differs from GitHub application scrolling.

## Decision

Accept `name` attributes on every HTML element, since seiso prefers a missed
diagnosis to a false one. Keep the exclusions for comments, escaped
tags, code, frontmatter, and nested text in raw HTML elements. Update the
independent oracle's extraction alongside the rule's documented anchors.

Only heading `name="config"` was tested in the live viewer. Accepting names
on other elements is a conservative allowance: it can miss a broken link in
another renderer. Site labels still require individual renderer review; the
GitHub-only oracle does not establish site precision.
