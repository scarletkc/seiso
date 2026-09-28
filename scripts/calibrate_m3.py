"""Compare existing duplicate thresholds on tuning inputs only; preserve all candidates."""

import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from evaluate_m2 import ROOT, digest, encode, fingerprints, prepare_inputs, read
from replay_m3 import build_probe, run_probe

CANDIDATES = {
    'baseline': {},
    'identifiers-3': {'min-identifiers': 3}, 'identifiers-7': {'min-identifiers': 7},
    'jaccard-70': {'min-jaccard': 0.7}, 'jaccard-90': {'min-jaccard': 0.9},
    'similarity-80': {'min-paragraph-similarity': 0.8}, 'similarity-95': {'min-paragraph-similarity': 0.95},
    'chars-40': {'min-paragraph-chars': 40}, 'chars-120': {'min-paragraph-chars': 120},
    'shingles-3': {'shingle-size': 3}, 'shingles-7': {'shingle-size': 7},
}


def run(output):
    if output.exists():
        raise ValueError(f'Preserve the existing calibration: {output}')
    corpus = ROOT / 'corpus'
    lock = read(corpus / 'corpus.lock.json')
    lock['sources'] = [source for source in lock['sources'] if source['split'] == 'tuning']
    inputs = prepare_inputs(corpus, lock, read(corpus / 'evaluation/kinds.json'), read(corpus / 'inventory/inventory.lock.json'))
    before = fingerprints()
    binary = build_probe('evaluate_m2')
    candidates = {}
    for name, settings in CANDIDATES.items():
        config = '\n[lint]\nselect=["DUP", "OWN"]\n[lint.dup]\n' + ''.join(f'{key}={value}\n' for key, value in settings.items())
        batch = {'sources': [source | {'config': source['config'] + config} for source in inputs['sources']]}
        files, raw = run_probe(binary, batch, ROOT / 'target/m3-calibration' / name)
        diagnoses = [{'source': file['source'], 'path': file['path'], 'sha256': file['sha256'], 'diagnostic': diagnostic} for file in files for diagnostic in file['result']['diagnostics']]
        candidates[name] = {'settings': settings, 'input_sha256': digest(encode(batch)), 'result_sha256': digest(raw), 'counts': dict(sorted(Counter(row['diagnostic']['code'] for row in diagnoses).items())), 'diagnostics': diagnoses}
        print(name, candidates[name]['counts'], flush=True)
    if before != fingerprints():
        raise ValueError('Implementation changed during calibration')
    report = {'schema_version': 1, 'split': 'tuning', 'sources': [source['id'] for source in lock['sources']], 'documents': sum(len(source['documents']) for source in lock['sources']),
        'corpus_sha256': digest((corpus / 'corpus.lock.json').read_bytes()), 'kind_profile_sha256': digest((corpus / 'evaluation/kinds.json').read_bytes()), 'inventory_sha256': digest((corpus / 'inventory/inventory.lock.json').read_bytes()),
        'implementation': before, 'probe_sha256': digest(binary.read_bytes()), 'script_sha256': digest(Path(__file__).read_bytes()), 'candidates': candidates,
        'decision': 'Pending review; diagnostic counts alone cannot justify a threshold change.'}
    output.mkdir(parents=True)
    (output / 'candidates.json.gz').write_bytes(gzip.compress(encode(report), mtime=0))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args().output)


if __name__ == '__main__':
    main()
