"""Review M2 link claims using pinned bytes, markdown-it-py and GitHub Slugger.

Run with markdown-it-py==4.0.0. Install github-slugger@2.0.0 in an ignored
directory with lifecycle scripts disabled, and pass its index.js via --slugger.
This oracle never imports or invokes the Rust parser or rule implementations.
"""

import argparse
from collections import Counter
import gzip
import hashlib
from html.parser import HTMLParser
import importlib.metadata
import json
from pathlib import Path
import posixpath
import re
import subprocess
from urllib.parse import unquote, urlsplit

from review_links import source_link

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "corpus"


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


class ExplicitAnchors(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.anchors = set()
        self.raw = None

    def handle_starttag(self, tag, attributes):
        if self.raw:
            return
        for key, value in attributes:
            if value is not None and key in {"id", "name"}:
                self.anchors.add(value)
        if tag in {"script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "plaintext"}:
            self.raw = tag

    def handle_startendtag(self, tag, attributes):
        self.handle_starttag(tag, attributes)
        self.handle_endtag(tag)

    def handle_endtag(self, tag):
        if self.raw == tag:
            self.raw = None


def visible_inline(tokens):
    parts = []
    for token in tokens or []:
        if token.type in {"text", "code_inline"}:
            parts.append(token.content)
        elif token.type in {"softbreak", "hardbreak"}:
            parts.append("\n")
        elif token.type == "image":
            parts.append(visible_inline(token.children))
    return "".join(parts)


def content_facts(raw):
    from markdown_it import MarkdownIt

    source = raw.decode("utf-8-sig")
    lines = source.splitlines(keepends=True)
    if lines and lines[0].strip() == "---":
        for index, line in enumerate(lines[1:], 1):
            if line.strip() in {"---", "..."}:
                lines[:index + 1] = ["\n"] * (index + 1)
                break
    body = "".join(lines)
    parser = MarkdownIt("commonmark", {"html": True}).enable(["table", "strikethrough"])
    tokens = parser.parse(body)
    headings = []
    destinations = []
    for index, token in enumerate(tokens):
        if token.type == "heading_open":
            headings.append({"text": visible_inline(tokens[index + 1].children), "line": token.map[0] + 1})
        for child in token.children or []:
            if child.type == "link_open":
                destinations.append(child.attrGet("href"))
            elif child.type == "image":
                destinations.append(child.attrGet("src"))
    explicit = ExplicitAnchors()
    explicit.feed(parser.renderer.render(tokens, parser.options, {}))
    return {"headings": headings, "html_anchors": sorted(explicit.anchors), "destinations": destinations}


def resolve(source, destination, entries):
    parsed = urlsplit(destination)
    if parsed.scheme or parsed.netloc:
        return None, None, "external"
    path = unquote(parsed.path, encoding="utf-8", errors="strict")
    fragment = unquote(parsed.fragment, encoding="utf-8", errors="strict")
    if any(character in path + fragment for character in "\0{}$<>"):
        return None, fragment, "template"
    target = source if not path else posixpath.normpath(path.lstrip("/") if path.startswith("/") else posixpath.join(posixpath.dirname(source), path))
    if target == ".." or target.startswith("../"):
        return target, fragment, "outside"
    by_path = {entry["path"]: entry for entry in entries}
    for parent in [target, *list(Path(target).parents)]:
        entry = by_path.get(str(parent).replace("\\", "/"))
        if entry and entry["mode"] in {"120000", "160000"}:
            return target, fragment, "indirect"
    entry = by_path.get(target)
    if target == "." or (entry and entry["type"] == "tree"):
        return target, fragment, "directory"
    return target, fragment, "file" if entry else "missing"


def review(candidate, slugger, corpus_lock=None, inventory=None):
    candidate_bytes = candidate.read_bytes()
    corpus_lock = corpus_lock if corpus_lock is not None else CORPUS / "corpus.lock.json"
    inventory = inventory if inventory is not None else CORPUS / "inventory/inventory.lock.json"
    corpus_bytes = corpus_lock.read_bytes()
    inventory_bytes = inventory.read_bytes()
    lock = json.loads(corpus_bytes)
    sources = {source["id"]: source for source in lock["sources"]}
    documents = {(source["id"], item["path"]): item for source in sources.values() for item in source["documents"]}
    inventory_lock = json.loads(inventory_bytes)
    if inventory_lock["corpus_lock_sha256"] != digest(corpus_bytes):
        raise ValueError("Inventory does not describe this corpus lock")
    records = {source["id"]: source for source in inventory_lock["sources"]}
    if len(records) != len(inventory_lock["sources"]) or records.keys() != sources.keys():
        raise ValueError("Inventory sources differ from corpus lock")
    trees = {}
    for source_id, record in records.items():
        if record["archive"] != f"{source_id}.json.gz":
            raise ValueError(f"Invalid inventory archive: {source_id}")
        payload = (inventory.parent / record["archive"]).read_bytes()
        if digest(payload) != record["sha256"]:
            raise ValueError(f"Inventory checksum mismatch: {source_id}")
        tree = json.loads(gzip.decompress(payload))
        for key in ["id", "repository", "commit", "tree"]:
            if tree[key] != sources[source_id][key] or record[key] != sources[source_id][key]:
                raise ValueError(f"Inventory identity mismatch: {source_id}/{key}")
        if record["entries"] != len(tree["entries"]):
            raise ValueError(f"Inventory entry count mismatch: {source_id}")
        trees[source_id] = tree["entries"]
    cache = {}

    def load(source, path):
        key = source, path
        if key not in cache:
            record = documents[key]
            raw = (CORPUS / "data/blobs" / record["git_blob"]).read_bytes()
            if digest(raw) != record["sha256"]:
                raise ValueError(f"Original source checksum mismatch: {key}")
            cache[key] = raw, content_facts(raw)
        return cache[key]

    labels = []
    for file in json.loads(candidate_bytes):
        for diagnostic in file["result"]["diagnostics"]:
            code = diagnostic["code"]
            if code not in {"LNK002", "PTR002"}:
                continue
            source, path = file["source"], file["path"]
            identity = {"source": source, "path": path, "code": code, "span": diagnostic["byte_range"], "input_sha256": file["sha256"]}
            raw, facts = load(source, path)
            if digest(raw) != file["sha256"]:
                raise ValueError("Candidate source identity changed")
            links = [link for link in file["links"] if link["span"] == diagnostic["byte_range"]]
            if len(links) != 1:
                raise ValueError("Candidate does not identify exactly one source link")
            link = links[0]
            verified, evidence = source_link(raw, link)
            independent_link = unquote(link["destination"]) in {unquote(value) for value in facts["destinations"]}
            target, fragment, status = resolve(path, link["destination"], trees[source])
            target_sha = None
            if (source, target) in documents:
                load(source, target)
                target_sha = documents[source, target]["sha256"]
            line = diagnostic["location"]["row"]
            lines = raw.decode("utf-8").splitlines()
            labels.append(identity | {"id": digest(encode(identity)), "split": sources[source]["split"],
                "destination": link["destination"], "target": target, "fragment": fragment,
                "target_sha256": target_sha, "physical_status": status,
                "source_verified": verified and independent_link, "source_evidence": evidence,
                "source_excerpt": "\n".join(lines[max(0, line - 2):line + 1]),
                "source_line": line, "reviewer_kind": "independent_agent_with_oracle",
                "review_method": "markdown-it-py source parsing; pinned Git tree resolution; github-slugger headings and HTMLParser attributes",
                "label": "uncertain", "reason": "Pending independent anchor evaluation.",
                "agent_context_reviewed": False})
    keys = sorted(cache)
    groups = [[heading["text"] for heading in cache[key][1]["headings"]] for key in keys]
    javascript = "import Slugger from " + json.dumps(slugger.resolve().as_uri()) + "; let raw=''; for await (const chunk of process.stdin) raw+=chunk; const groups=JSON.parse(raw); console.log(JSON.stringify(groups.map(group=>{const slugger=new Slugger();return group.map(text=>slugger.slug(text));})));"
    outcome = subprocess.run(["node", "--input-type=module", "-e", javascript], input=json.dumps(groups).encode(), capture_output=True, check=True)
    for key, slugs in zip(keys, json.loads(outcome.stdout), strict=True):
        facts = cache[key][1]
        facts["anchors"] = sorted(set(slugs) | set(facts["html_anchors"]))
        for heading, slug in zip(facts["headings"], slugs, strict=True):
            heading["slug"] = slug
    for row in labels:
        if not row["source_verified"]:
            row["reason"] = "Original source and independent parser need individual adjudication."
        elif row["code"] == "PTR002":
            row["label"] = "tp" if row["physical_status"] == "directory" else "fp"
            row["reason"] = "Pinned Git tree confirms a directory; the frozen evaluation configuration declares no catalog directories." if row["label"] == "tp" else "Pinned target is not a directory."
        elif row["physical_status"] != "file" or row["target_sha256"] is None:
            row["reason"] = "Target is not a directly available pinned Markdown file."
        else:
            raw, facts = cache[row["source"], row["target"]]
            row["oracle_anchors"] = facts["anchors"]
            row["oracle_headings"] = facts["headings"]
            row["oracle_html_anchors"] = facts["html_anchors"]
            row["label"] = "fp" if row["fragment"] in facts["anchors"] else "tp"
            row["reason"] = "Independent GitHub heading slugs and explicit HTML anchors " + ("contain" if row["label"] == "fp" else "do not contain") + f" the exact decoded fragment {row['fragment']!r}."
            custom = "{#" + row["fragment"] + "}"
            row["custom_renderer_anchor"] = custom if custom in raw.decode("utf-8") else None
    package = json.loads(slugger.with_name("package.json").read_bytes())
    if package["version"] != "2.0.0" or importlib.metadata.version("markdown-it-py") != "4.0.0":
        raise ValueError("Oracle dependency versions changed")
    return {"schema_version": 1, "candidate_sha256": digest(candidate_bytes), "corpus_sha256": digest(corpus_bytes),
        "inventory_sha256": digest(inventory_bytes), "human_reviewers": 0,
        "scope": "Physical GitHub-only contract oracle; labels require separate context adjudication before use as actionable precision measurements.",
        "oracle": {"markdown-it-py": "4.0.0", "github-slugger": "2.0.0", "slugger_index_sha256": digest(slugger.read_bytes()), "slugger_regex_sha256": digest(slugger.with_name("regex.js").read_bytes())},
        "labels": labels}


def bind_decisions(report_path, decisions_path):
    compressed = report_path.read_bytes()
    report = json.loads(gzip.decompress(compressed))
    decisions = json.loads(decisions_path.read_bytes())
    for key in ["corpus_sha256", "inventory_sha256"]:
        if report[key] != decisions[key]:
            raise ValueError(f"Review inputs differ from the frozen report: {key}")
    expected = {row["id"]: row for row in report["diagnostics"] if row["code"] in decisions["scope_codes"]}
    labels = decisions["labels"]
    if len({row["id"] for row in labels}) != len(labels) or {row["id"] for row in labels} != set(expected):
        raise ValueError("Review is incomplete, duplicated, or refers to other diagnostics")
    for label in labels:
        diagnostic = expected[label["id"]]
        for key in ["code", "source", "path", "span", "input_sha256", "related_inputs"]:
            if label[key] != diagnostic[key]:
                raise ValueError(f"Diagnostic identity changed: {label['id']} / {key}")
        if label["diagnostic_sha256"] != digest(encode(diagnostic["diagnostic"])):
            raise ValueError(f"Diagnostic payload changed: {label['id']}")
        if label["label"] not in {"tp", "fp", "uncertain"} or not label.get("reason"):
            raise ValueError("Every reviewed diagnostic needs a valid label and reason")
    return decisions | {"report_sha256": digest(compressed), "decision_receipt_sha256": digest(decisions_path.read_bytes())}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--candidate", type=Path)
    mode.add_argument("--bind-report", type=Path)
    parser.add_argument("--slugger", type=Path)
    parser.add_argument("--corpus-lock", type=Path, help="Pinned corpus lock; defaults to the main corpus")
    parser.add_argument("--inventory", type=Path, help="Inventory lock; archives are relative to this file")
    parser.add_argument("--decisions", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.candidate:
        if args.slugger is None:
            parser.error("--candidate requires --slugger")
        result = review(args.candidate, args.slugger, args.corpus_lock, args.inventory)
    else:
        if args.decisions is None:
            parser.error("--bind-report requires --decisions")
        result = bind_decisions(args.bind_report, args.decisions)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encode(result))
    print(json.dumps(dict(Counter(f"{row['split']}/{row['code']}/{row['label']}" for row in result["labels"])), sort_keys=True))


if __name__ == "__main__":
    main()
