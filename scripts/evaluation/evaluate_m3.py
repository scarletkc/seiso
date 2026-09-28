"""Freeze heuristic diagnostics and section predictions with isolated tuning/holdout runs."""

import argparse
from pathlib import Path

from .evaluate_m2 import ROOT, run

RULES = ("STL002", "STL004", "RAT001", "ORD001", "ORD002", "MIX001", "VOX002", "VOX003", "EVD001")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--corpus-dir", type=Path, default=ROOT / "corpus")
    parser.add_argument("--split", choices=["tuning", "holdout", "all"], required=True)
    args = parser.parse_args()
    run(args.output, args.corpus_dir, None if args.split == "all" else args.split, sections=True)


if __name__ == "__main__":
    main()
