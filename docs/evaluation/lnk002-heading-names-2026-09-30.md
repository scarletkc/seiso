---
kind: adr
---

# LNK002 heading name decision

Recorded on 2026-09-30 for [issue 11](https://github.com/scarletkc/seiso/issues/11).
LNK002 treats a `name` attribute on any HTML element as an anchor, not only
on `<a>`.

## Evidence

ripgrep's `FAQ.md` links to `#config`, and at commit `3fce3b5` the only
matching element is `<h3 name="config">`. On GitHub's page for that commit,
`#config` scrolls the heading into view just as its ordinary slug does, and a
fragment that matches nothing leaves the page at the top. The
[browser measurements](../../corpus/results/lnk002/name-attribute/browser.json)
and [screenshot](../../corpus/results/lnk002/name-attribute/name-config.png)
record this, and
[`check_github.py`](../../corpus/results/lnk002/name-attribute/check_github.py)
repeats the check.

## Decision

Count `name` on every element. Only a heading was checked on GitHub, and the
[HTML Standard](https://html.spec.whatwg.org/multipage/browsing-the-web.html#the-indicated-part-of-the-document)
matches only `id` and `<a name>`, so LNK002 can miss a broken link in some
renderers. seiso prefers that to a false report.
