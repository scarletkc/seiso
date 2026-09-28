"""Validate M3 natural labels and change replay labels without promoting heuristics."""

import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from .evaluate_m2 import ROOT, digest, encode, read
from .evaluate_m3 import RULES
from .summarize_m2 import dimensions, validate_labels, verify_sources


def validate_replay(replay):
    documents = {row['id']: row for row in replay['documents']}
    results = {row['id']: row for row in replay['results']}
    rows = {row['id']: row for row in replay['diagnostics']}
    if len(documents) != len(replay['documents']) or len(results) != len(replay['results']) or len(rows) != len(replay['diagnostics']):
        raise ValueError('Duplicate replay identity')
    for document in documents.values():
        if digest(document['source'].encode()) != document['source_sha256']:
            raise ValueError('Replay source hash mismatch')
    covered = []
    for change in replay['changes']:
        if 'after_diagnostics' not in change:
            raise ValueError('Replay must retain all post-change diagnostics to measure persistent blocking')
        after = change['after']
        ids = change['after_diagnostics']
        if results[after]['source_sha256'] != documents[after]['source_sha256']:
            raise ValueError('Replay result input mismatch')
        if [rows[identity]['diagnostic'] for identity in ids] != results[after]['result']['diagnostics']:
            raise ValueError('Post-change diagnostics were omitted or changed')
        for identity in ids:
            row = rows[identity]
            if row['code'] not in RULES or row['change_id'] != change['id'] or row['document_id'] != after or row['input_sha256'] != documents[after]['source_sha256']:
                raise ValueError('Replay diagnostic input mismatch')
        covered.extend(ids)
    if len(covered) != len(set(covered)) or set(covered) != rows.keys():
        raise ValueError('Incomplete or duplicated post-change diagnostic coverage')
    return rows


def noise(changes, rows, decisions):
    change_ids = {change['id'] for change in changes}
    if len(change_ids) != len(changes) or any(row['change_id'] not in change_ids for row in rows.values()):
        raise ValueError('Duplicate or missing change identity')
    result = {}
    for code in RULES:
        selected = [row for row in rows.values() if row['code'] == code]
        fp = {row['change_id'] for row in selected if decisions[row['id']]['label'] == 'fp'}
        uncertain = {row['change_id'] for row in selected if decisions[row['id']]['label'] == 'uncertain'}
        affected = {row['change_id'] for row in selected}
        result[code] = {'changes': len(changes), 'diagnostics': len(selected), 'changes_with_diagnostics': len(affected),
            'changes_with_new_diagnostics': len({row['change_id'] for row in selected if row.get('introduced', False)}),
            'incorrect_blocks': len(fp), 'uncertain_blocks': len(uncertain),
            'incorrect_blocks_per_100_changes': 100 * len(fp) / len(changes) if changes else None,
            'conservative_blocks_per_100_changes': 100 * len(fp | uncertain) / len(changes) if changes else None,
            'diagnostic_labels': {label: sum(decisions[row['id']]['label'] == label for row in selected) for label in ['tp', 'fp', 'uncertain']},
            'promotion': False, 'status': 'preview'}
    return result


def run(report_path, labels_path, replay_path, replay_labels, output, corpus):
    if output.exists():
        raise ValueError(f'Preserve the existing summary: {output}')
    packed = report_path.read_bytes()
    report = json.loads(gzip.decompress(packed))
    if report.get('evaluation_split') != 'all':
        raise ValueError('M3 summary requires a full tuning and holdout run')
    selected = [row for row in report['diagnostics'] if row['code'] in RULES]
    rows = {row['id']: row for row in selected}
    if len(rows) != len(selected):
        raise ValueError('Duplicate natural diagnostic identity')
    verify_sources(rows, corpus, report['corpus_sha256'])
    decisions, provenance = validate_labels([(labels_path.name, read(labels_path))], rows, digest(packed))
    replay_raw = replay_path.read_bytes()
    replay = json.loads(gzip.decompress(replay_raw))
    replay_rows = validate_replay(replay)
    replay_decisions, _ = validate_labels([(replay_labels.name, read(replay_labels))], replay_rows, digest(replay_raw))
    for path, sha in report['implementation'].items():
        if path.startswith('docs/rules/'):
            continue
        if replay['implementation'].get(path) != sha:
            raise ValueError('Natural evaluation and replay use different implementations')
    natural = {}
    for code in RULES:
        selected = [row for row in rows.values() if row['code'] == code]
        natural[code] = {'status': 'preview', 'promotion': False,
            'splits': {split: dimensions([row for row in selected if row['split'] == split], decisions) for split in ['tuning', 'holdout']}}
    result = {'schema_version': 1, 'report_sha256': digest(packed), 'labels_sha256': digest(labels_path.read_bytes()),
        'replay_sha256': digest(replay_raw), 'replay_labels_sha256': digest(replay_labels.read_bytes()), 'script_sha256': digest(Path(__file__).read_bytes()),
        'rules': natural, 'usage_noise': noise(replay['changes'], replay_rows, replay_decisions),
        'reviewer_kind': read(labels_path)['reviewer_kind'], 'replay_reviewer_kind': read(replay_labels)['reviewer_kind'],
        'gate': {'all_proposed_rule_reports_present': set(natural) == set(RULES), 'rules_promoted': [], 'human_meaning_review': 'not performed',
                 'independent_agent_repair_trial': 'not performed', 'recall': 'section sample reports classification misses; rule recall is not estimated'},
        'noise_definition': 'A block is one real Markdown path change with at least one false-positive diagnostic after checking, whether newly introduced or persistent. Each change counts once per rule; all clean changes remain in the denominator. Newly introduced diagnostics are reported separately.',
        'limitations': ['Replay covers one repository under a fixed kind profile, not original historical configurations.', 'Agent judgments are not human labels.', 'Zero diagnostics imply unavailable precision, not perfect precision.']}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(encode(result))
    print(json.dumps({code: row['splits']['holdout'] for code, row in natural.items()}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['report', 'labels', 'replay', 'replay-labels', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--corpus-dir', type=Path, default=ROOT / 'corpus')
    args = parser.parse_args()
    run(args.report, args.labels, args.replay, args.replay_labels, args.output, args.corpus_dir)


if __name__ == '__main__':
    main()
