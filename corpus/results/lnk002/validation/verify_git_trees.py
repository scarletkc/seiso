#!/usr/bin/env python3
"""Reproduce the independent stage 3 Git-tree reconstruction check.

Reads only the batch inventory lock and its compressed tree inventories.
Does not read document blobs, diagnostics, labels, or upstream executable code.
Uses Python's standard library and writes no files.

The reconstruction algorithm is the same one used in the independent review:
group recursive entries by immediate parent, serialize each parent's direct
children as a Git tree, hash that tree, and compare it to the pinned tree SHA-1.
"""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "batch_dir",
        nargs="?",
        type=Path,
        default=Path("/workspace/seiso/corpus/results/lnk002/fresh-v1/batch-1"),
        help="batch directory containing inventory/inventory.lock.json",
    )
    args = parser.parse_args()
    inventory_dir = args.batch_dir / "inventory"
    inventory_lock = json.loads(
        (inventory_dir / "inventory.lock.json").read_text()
    )
    errors = []
    tree_count = 0
    entry_count = 0
    source_results = []

    for source in inventory_lock["sources"]:
        inventory = json.loads(
            gzip.decompress((inventory_dir / source["archive"]).read_bytes())
        )
        entry_count += len(inventory["entries"])
        children = {}
        for entry in inventory["entries"]:
            path = Path(entry["path"])
            parent = path.parent.as_posix()
            parent = "" if parent == "." else parent
            children.setdefault(parent, []).append(entry)

        source_tree_count = 0
        source_errors = []
        for parent, entries in children.items():
            def sort_key(entry):
                # Git compares directory names as if they end with '/'.
                return Path(entry["path"]).name.encode() + (
                    b"/" if entry["type"] == "tree" else b""
                )

            # Git's tree object stores unpadded octal mode, a space, the UTF-8
            # basename, NUL, then the child's 20-byte binary object ID.
            raw = b"".join(
                (entry["mode"].lstrip("0") + " " + Path(entry["path"]).name)
                .encode()
                + b"\0"
                + bytes.fromhex(entry["sha"])
                for entry in sorted(entries, key=sort_key)
            )
            actual = hashlib.sha1(
                b"tree " + str(len(raw)).encode() + b"\0" + raw
            ).hexdigest()
            expected = (
                source["tree"]
                if not parent
                else next(
                    (
                        entry["sha"]
                        for entry in inventory["entries"]
                        if entry["path"] == parent
                    ),
                    None,
                )
            )
            tree_count += 1
            source_tree_count += 1
            if actual != expected:
                mismatch = {
                    "source": source["id"],
                    "path": parent,
                    "expected": expected,
                    "actual": actual,
                }
                errors.append(mismatch)
                source_errors.append(mismatch)

        source_results.append(
            {
                "source": source["id"],
                "root_tree": source["tree"],
                "inventory_entries": len(inventory["entries"]),
                "reconstructed_trees": source_tree_count,
                "mismatches": len(source_errors),
            }
        )

    print(
        json.dumps(
            {
                "algorithm": "git-tree-sha1-from-recursive-inventory",
                "inventory_entries": entry_count,
                "reconstructed_trees": tree_count,
                "sources": source_results,
                "errors": errors,
            },
            indent=2,
        )
    )
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
