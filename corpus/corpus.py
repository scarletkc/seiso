"""Freeze, fetch, and verify a commit-pinned Markdown corpus without running upstream code."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import fnmatch
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent
LOCK = ROOT / "corpus.lock.json"


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path, value):
    content = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
        temporary.write(content)
        name = Path(temporary.name)
    name.replace(path)


def github(route):
    process = subprocess.run(["gh", "api", route], capture_output=True, check=True)
    return json.loads(process.stdout)


def valid_path(value):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or "\\" in value or any(part in {".", ".."} for part in path.parts):
        raise ValueError(f"Unsafe corpus path: {value!r}")
    return path


def blob_hash(content):
    return hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest()


def resolve_source(source, *, with_tree=False):
    """Resolve once; optionally return the exact recursive tree used for selection."""
    repository = github(f"repos/{source['repository']}")
    if repository["private"]:
        raise ValueError(f"Corpus source must be public: {source['repository']}")
    name = repository["full_name"]
    commit = github(f"repos/{name}/commits/{quote(repository['default_branch'], safe='')}")
    tree = github(f"repos/{name}/git/trees/{commit['commit']['tree']['sha']}?recursive=1")
    if tree.get("truncated"):
        raise ValueError(f"GitHub returned a truncated tree for {name}; cannot prove complete selection")
    documents = []
    matched = {pattern: 0 for pattern in source["include"]}
    regular_files = [entry for entry in tree["tree"] if entry["type"] == "blob" and entry["mode"] in {"100644", "100755"}]
    for entry in regular_files:
        path = entry["path"]
        if PurePosixPath(path).suffix.lower() not in {".md", ".markdown"}:
            continue
        patterns = [pattern for pattern in matched if fnmatch.fnmatchcase(path, pattern)]
        if patterns:
            valid_path(path)
            for pattern in patterns:
                matched[pattern] += 1
            documents.append({"path": path, "git_blob": entry["sha"], "bytes": entry["size"]})
    if not documents:
        raise ValueError(f"No Markdown documents matched {name}: {source['include']}")
    ancestors = {str(parent) for entry in documents for parent in PurePosixPath(entry["path"]).parents}
    licenses = [
        {"path": entry["path"], "git_blob": entry["sha"], "bytes": entry["size"]}
        for entry in regular_files
        if entry["path"] in source.get("license_paths", []) or (
            str(PurePosixPath(entry["path"]).parent) in ancestors
            and re.match(r"^(license|copying|notice)([._-].*)?$", PurePosixPath(entry["path"]).name, re.IGNORECASE)
        )
    ]
    if not licenses:
        raise ValueError(f"No license/notice file found for {name}; review before adding it")
    result = dict(source)
    result.update(repository=name, commit=commit["sha"], tree=commit["commit"]["tree"]["sha"],
                  license_hint=(repository.get("license") or {}).get("spdx_id"),
                  documents=sorted(documents, key=lambda entry: entry["path"]),
                  licenses=sorted(licenses, key=lambda entry: entry["path"]), matched_patterns=matched)
    return (result, tree) if with_tree else result


def fetch_blob(source, entry):
    valid_path(entry["path"])
    path = ROOT / "data" / "blobs" / entry["git_blob"]
    if path.is_file():
        content = path.read_bytes()
        if blob_hash(content) == entry["git_blob"]:
            return hashlib.sha256(content).hexdigest()
    url = f"https://raw.githubusercontent.com/{source['repository']}/{source['commit']}/{quote(entry['path'], safe='/')}"
    for attempt in range(4):
        try:
            with urlopen(Request(url, headers={"User-Agent": "seiso-corpus/1"}), timeout=60) as response:
                content = response.read()
            break
        except (HTTPError, URLError, TimeoutError) as error:
            if attempt == 3 or isinstance(error, HTTPError) and error.code not in {408, 429, 500, 502, 503, 504}:
                raise RuntimeError(f"Cannot fetch {source['id']}/{entry['path']}: {error}") from error
            time.sleep(2 ** attempt)
    if len(content) != entry["bytes"] or blob_hash(content) != entry["git_blob"]:
        raise ValueError(f"Git blob mismatch for {source['id']}/{entry['path']}")
    if "sha256" in entry and hashlib.sha256(content).hexdigest() != entry["sha256"]:
        raise ValueError(f"SHA-256 mismatch for {source['id']}/{entry['path']}")
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
        temporary.write(content)
        name = Path(temporary.name)
    name.replace(path)
    return hashlib.sha256(content).hexdigest()


def validate_lock(lock):
    if lock.get("schema_version") != 1 or not lock.get("sources"):
        raise ValueError("Unsupported or empty corpus lock")
    ids = set()
    repositories = set()
    for source in lock["sources"]:
        if not re.fullmatch(r"[a-z0-9-]+", source["id"]) or source["id"] in ids:
            raise ValueError("Corpus source IDs must be unique safe directory names")
        ids.add(source["id"])
        if source["repository"].lower() in repositories:
            raise ValueError("A repository cannot occur in both corpus splits")
        repositories.add(source["repository"].lower())
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", source["repository"]):
            raise ValueError("Invalid GitHub repository name")
        if not re.fullmatch(r"[a-f0-9]{40}", source["commit"]):
            raise ValueError("Corpus revisions must be full immutable commit hashes")
        paths = set()
        for entry in source["documents"] + source["licenses"]:
            valid_path(entry["path"])
            if not re.fullmatch(r"[a-f0-9]{40}", entry["git_blob"]):
                raise ValueError("Invalid Git blob identifier")
        for entry in source["documents"]:
            if entry["path"] in paths:
                raise ValueError("Duplicate document in corpus lock")
            paths.add(entry["path"])
        if not paths:
            raise ValueError("Empty source in corpus lock")


def download(lock, freeze=False):
    validate_lock(lock)
    entries_by_blob = {}
    for source in lock["sources"]:
        for entry in source["documents"] + source["licenses"]:
            entries_by_blob.setdefault(entry["git_blob"], []).append((source, entry))
    with ThreadPoolExecutor(max_workers=12) as executor:
        by_future = {
            executor.submit(fetch_blob, *entries[0]): entries
            for entries in entries_by_blob.values()
        }
        total = len(by_future)
        for count, future in enumerate(as_completed(by_future), 1):
            digest = future.result()
            for source, entry in by_future[future]:
                if freeze:
                    entry["sha256"] = digest
                elif entry.get("sha256") != digest:
                    raise ValueError(f"SHA-256 mismatch for {source['id']}/{entry['path']}")
            if count % 100 == 0 or count == total:
                print(f"Verified {count}/{total} content files", flush=True)


def freeze(resume=False):
    if LOCK.exists():
        raise ValueError("A corpus lock already exists; preserve it and create a separately reviewed revision")
    sources = read_json(ROOT / "sources.json")["sources"]
    pending = ROOT / "selection.pending.json"
    cached = {source["id"]: source for source in read_json(pending)["sources"]} if resume and pending.exists() else {}
    def resolve(source):
        previous = cached.get(source["id"])
        if previous and all(previous.get(key) == value for key, value in source.items()):
            return previous
        return resolve_source(source)
    resolved = []
    with ThreadPoolExecutor(max_workers=4) as executor:
        for source in executor.map(resolve, sources):
            resolved.append(source)
            write_json(ROOT / "selection.pending.json", {"schema_version": 1, "sources": resolved})
            print(f"{source['id']}: {len(source['documents'])} documents at {source['commit']}; {source['matched_patterns']}", flush=True)
    lock = {"schema_version": 1, "selection": "All regular .md/.markdown files matching each source's declared include patterns at its pinned commit", "sources": resolved}
    write_json(ROOT / "selection.pending.json", lock)
    download(lock, freeze=True)
    write_json(LOCK, lock)
    print(f"Wrote {LOCK.name}", flush=True)


def verify():
    lock = read_json(LOCK)
    validate_lock(lock)
    count = 0
    for source in lock["sources"]:
        for entry in source["documents"] + source["licenses"]:
            path = ROOT / "data" / "blobs" / entry["git_blob"]
            content = path.read_bytes()
            if len(content) != entry["bytes"] or blob_hash(content) != entry["git_blob"] or hashlib.sha256(content).hexdigest() != entry["sha256"]:
                raise ValueError(f"Content mismatch: {source['id']}/{entry['path']}")
        count += len(source["documents"])
    print(f"Verified {count} Markdown files from {len(lock['sources'])} pinned sources", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["freeze", "fetch", "verify"])
    parser.add_argument("--resume", action="store_true", help="Reuse pinned selections after an interrupted freeze")
    args = parser.parse_args()
    if args.command == "freeze":
        freeze(args.resume)
    elif args.command == "fetch":
        download(read_json(LOCK))
        verify()
    else:
        verify()
