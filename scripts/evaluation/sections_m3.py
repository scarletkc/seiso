"""Sample and score source-bound section annotations, including unclassified sections."""

import argparse
from collections import Counter, defaultdict
import gzip
import json
from pathlib import Path

from .evaluate_m2 import ROOT, digest, encode, read, verify_blob

TYPES = {'steps', 'reference', 'rationale', 'background', 'troubleshooting', 'other'}


def sample(report_path, corpus, lock_path=None):
    packed = report_path.read_bytes()
    report = json.loads(gzip.decompress(packed))
    lock_path = lock_path or corpus / 'corpus.lock.json'
    lock = read(lock_path)
    if digest(lock_path.read_bytes()) != report['corpus_sha256']:
        raise ValueError('Section corpus binding mismatch')
    sources = {source['id']: source for source in lock['sources']}
    documents = {(source['id'], document['path']): document for source in lock['sources'] for document in source['documents']}
    groups, splits = defaultdict(list), defaultdict(list)
    for file in report['files']:
        record = documents[(file['source'], file['path'])]
        blob = corpus / 'data/blobs' / record['git_blob']
        verify_blob(blob, record)
        raw = blob.read_bytes()
        for annotation in file['section_annotations']:
            span = annotation['content_span']
            if annotation['section'] == 0 or not raw[span['start']:span['end']].strip():
                continue
            identity = {'source': file['source'], 'path': file['path'], 'input_sha256': file['sha256'], 'section': annotation['section']}
            row = identity | {'id': digest(encode(identity)), 'annotation': annotation, 'annotation_sha256': digest(encode(annotation)),
                'git_blob': record['git_blob'], 'split': sources[file['source']]['split'], 'language': file['language']}
            groups[(row['split'], row['language'], annotation['section_type'])].append(row)
            splits[row['split']].append(row)
    chosen = {}
    for rows in groups.values():
        row = min(rows, key=lambda row: row['id'])
        chosen[row['id']] = row | {'selection': 'first hash in split/language/predicted-type stratum'}
    for rows in splits.values():
        for row in sorted(rows, key=lambda row: row['id'])[:6]:
            chosen.setdefault(row['id'], row | {'selection': 'first six hashes in split independent of predicted type'})
    return {'schema_version': 1, 'report_sha256': digest(packed), 'corpus_sha256': report['corpus_sha256'],
        'method': 'One nonempty headed section per split/language/predicted-type stratum, plus six per split by hash without type selection. All document kinds included. Root sections excluded.',
        'population': len({row['id'] for rows in groups.values() for row in rows}), 'samples': sorted(chosen.values(), key=lambda row: row['id'])}


def summarize(sample_path, labels_path, report_path, corpus, lock_path=None):
    expected = sample(report_path, corpus, lock_path)
    frozen = read(sample_path)
    if frozen != expected:
        raise ValueError('Section sample does not match the report or source bytes')
    labels = read(labels_path)
    if labels.get('sample_sha256') != digest(sample_path.read_bytes()) or not labels.get('reviewer_kind'):
        raise ValueError('Section label binding or reviewer missing')
    rows = {row['id']: row for row in frozen['samples']}
    decisions = {}
    for label in labels['labels']:
        identity = label['id']
        if identity not in rows or identity in decisions:
            raise ValueError('Unknown or duplicate section label')
        row = rows[identity]
        if label.get('input_sha256') != row['input_sha256'] or label.get('annotation_sha256') != row['annotation_sha256']:
            raise ValueError('Section label source or prediction mismatch')
        if label.get('expected_type') not in TYPES | {'uncertain'} or not label.get('reason', '').strip():
            raise ValueError('Invalid section judgment')
        decisions[identity] = label
    if decisions.keys() != rows.keys():
        raise ValueError('Missing section labels')
    confusion = Counter((rows[key]['annotation']['section_type'], label['expected_type']) for key, label in decisions.items())
    misses = [key for key, label in decisions.items() if rows[key]['annotation']['section_type'] == 'other' and label['expected_type'] not in {'other', 'uncertain'}]
    return {'schema_version': 1, 'sample_sha256': digest(sample_path.read_bytes()), 'labels_sha256': digest(labels_path.read_bytes()),
        'reviewer_kind': labels['reviewer_kind'], 'sample_count': len(rows), 'population': frozen['population'],
        'confusion': [{'predicted': predicted, 'reviewed': gold, 'count': count} for (predicted, gold), count in sorted(confusion.items())],
        'false_negative_ids': misses, 'uncertain': sum(label['expected_type'] == 'uncertain' for label in decisions.values()),
        'scope': 'A stratified classification audit, not a representative accuracy or rule-recall estimate.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['sample', 'summarize'])
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--corpus-dir', type=Path, default=ROOT / 'corpus')
    parser.add_argument('--corpus-lock', type=Path)
    parser.add_argument('--sample', type=Path)
    parser.add_argument('--labels', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError(f'Preserve the existing section artifact: {args.output}')
    if args.command == 'summarize' and (args.sample is None or args.labels is None):
        parser.error('summarize requires --sample and --labels')
    result = sample(args.report, args.corpus_dir, args.corpus_lock) if args.command == 'sample' else summarize(args.sample, args.labels, args.report, args.corpus_dir, args.corpus_lock)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encode(result))


if __name__ == '__main__':
    main()
