"""Replay frozen M3 natural and history outcomes against the current implementation."""

import argparse
import gzip
import json
from pathlib import Path

from evaluate_m2 import ROOT, digest, encode, fingerprints, prepare_inputs, read, reverse_inputs
from replay_m3 import build_probe, introduced, run_probe


def verify(natural_path, history_path):
    natural = json.loads(gzip.decompress(natural_path.read_bytes()))
    history = json.loads(gzip.decompress(history_path.read_bytes()))
    corpus = ROOT / 'corpus'
    for relative, key in [('corpus.lock.json', 'corpus_sha256'), ('evaluation/kinds.json', 'kind_profile_sha256'), ('inventory/inventory.lock.json', 'inventory_sha256')]:
        if digest((corpus / relative).read_bytes()) != natural[key]:
            raise ValueError(f'Frozen input changed: {relative}')
    inputs = prepare_inputs(corpus, read(corpus / 'corpus.lock.json'), read(corpus / 'evaluation/kinds.json'), read(corpus / 'inventory/inventory.lock.json')) | {'sections': True}
    before = fingerprints()
    binary = build_probe('evaluate_m2')
    files, raw = run_probe(binary, inputs, ROOT / 'target/m3-verification/natural')
    _, reverse = run_probe(binary, reverse_inputs(inputs), ROOT / 'target/m3-verification/natural-reverse')
    if files != natural['files'] or digest(raw) != natural['raw_result_sha256'] or raw != reverse:
        raise ValueError('Current natural diagnostics or section predictions differ from the frozen evidence')
    replay_binary = build_probe('replay_m3')
    results, raw = run_probe(replay_binary, {'config': history['policy'], 'documents': history['documents']}, ROOT / 'target/m3-verification/history')
    _, reverse = run_probe(replay_binary, {'config': history['policy'], 'documents': list(reversed(history['documents']))}, ROOT / 'target/m3-verification/history-reverse')
    if results != history['results'] or raw != reverse:
        raise ValueError('Current history diagnostics or annotations differ from the frozen evidence')
    documents = {row['id']: row for row in history['documents']}
    by_id = {row['id']: row for row in results}
    rows = {row['id']: row for row in history['diagnostics']}
    for change in history['changes']:
        old, new = (by_id[change[side]]['result']['diagnostics'] for side in ['before', 'after'])
        if 'after_diagnostics' in change and new != [rows[identity]['diagnostic'] for identity in change['after_diagnostics']]:
            raise ValueError('Post-change diagnostics differ from the frozen history')
        added = introduced(old, new, documents[change['before']]['source'], documents[change['after']]['source'])
        if added != [rows[identity]['diagnostic'] for identity in change['introduced']]:
            raise ValueError('Introduced diagnostic matching differs from the frozen history')
    if before != fingerprints():
        raise ValueError('Implementation changed during verification')
    return {'schema_version': 1, 'natural_sha256': digest(natural_path.read_bytes()), 'history_sha256': digest(history_path.read_bytes()),
        'implementation': before, 'script_sha256': digest(Path(__file__).read_bytes()), 'natural_probe_sha256': digest(binary.read_bytes()),
        'history_probe_sha256': digest(replay_binary.read_bytes()), 'natural_files_identical': True, 'history_results_identical': True,
        'introduced_diagnostics_identical': True, 'reverse_byte_identical': True,
        'changes_since_natural_freeze': {path: {'frozen_sha256': natural['implementation'].get(path), 'current_sha256': sha} for path, sha in before.items() if natural['implementation'].get(path) != sha}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--natural', type=Path, required=True)
    parser.add_argument('--history', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError(f'Preserve the existing verification: {args.output}')
    result = verify(args.natural, args.history)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encode(result))
    print('Verified identical natural, section, history, and introduced diagnostic results.')


if __name__ == '__main__':
    main()
