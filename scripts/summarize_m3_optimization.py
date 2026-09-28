"""Bind M3 optimization evidence while separating reused data from fresh evaluation."""

import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from evaluate_m2 import ROOT, digest, encode, fingerprints, read, verify_blob
from evaluate_m3 import RULES
from summarize_m2 import dimensions, validate_labels
from summarize_m3 import noise, validate_replay


def report(path):
    return json.loads(gzip.decompress(path.read_bytes()))


def labeled(path, labels_path, selected_rules=True):
    value = report(path)
    selected = [row for row in value['diagnostics'] if not selected_rules or row['code'] in RULES]
    rows = {row['id']: row for row in selected}
    if len(rows) != len(selected):
        raise ValueError('Duplicate diagnostic identity')
    decisions, _ = validate_labels([(labels_path.name, read(labels_path))], rows, digest(path.read_bytes()))
    return value, rows, decisions


def compare_cohort(before, after):
    old = {(file['source'], file['path']): (file['sha256'], file['result']['kind']) for file in before['files']}
    new = {(file['source'], file['path']): (file['sha256'], file['result']['kind']) for file in after['files']}
    if old != new:
        raise ValueError('Comparison changed source bytes, file coverage, or effective kinds')


def score_sections(sample, labels, files):
    by_file = {(file['source'], file['path']): file for file in files}
    by_id = {row['id']: row for row in labels['labels']}
    rows = []
    for item in sample['samples']:
        file = by_file[(item['source'], item['path'])]
        label = by_id[item['id']]
        if file['sha256'] != item['input_sha256'] or label['input_sha256'] != item['input_sha256'] or label['annotation_sha256'] != item['annotation_sha256']:
            raise ValueError('Section review input or original-prediction binding changed')
        predicted = file['section_annotations'][item['section']]['section_type']
        rows.append({'id': item['id'], 'before': item['annotation']['section_type'], 'after': predicted, 'reviewed': label['expected_type']})
    return {'samples': len(rows), 'before_matches': sum(row['before'] == row['reviewed'] for row in rows),
            'after_matches': sum(row['after'] == row['reviewed'] for row in rows), 'rows': rows}


def run(base, output):
    if output.exists():
        raise ValueError(f'Preserve the existing summary: {output}')
    current = base / 'optimization-v2'
    known, known_rows, known_labels = labeled(current / 'known-corpus/diagnostics.json.gz', current / 'known-corpus/labels.json')
    original, old_rows, old_labels = labeled(base / 'natural-v1/diagnostics.json.gz', base / 'natural-v1/labels.json')
    compare_cohort(original, known)
    non_m3_old = {row['id']: row['diagnostic'] for row in original['diagnostics'] if row['code'] not in RULES}
    non_m3_new = {row['id']: row['diagnostic'] for row in known['diagnostics'] if row['code'] not in RULES}
    if non_m3_old != non_m3_new:
        raise ValueError('Existing-rule diagnostics changed; review before summarizing optimization')
    first, first_rows, first_labels = labeled(base / 'optimization-v1/fresh-holdout/diagnostics.json.gz', base / 'optimization-v1/fresh-holdout/labels.json')
    replay, replay_rows, replay_labels = labeled(current / 'first-cohort-replay/diagnostics.json.gz', current / 'first-cohort-replay/labels.json')
    compare_cohort(first, replay)
    fresh, fresh_rows, fresh_labels = labeled(current / 'fresh-complete/diagnostics.json.gz', current / 'fresh-complete/labels.json')
    initial_fresh = report(current / 'fresh-holdout/diagnostics.json.gz')
    stripped = [{key: value for key, value in file.items() if key != 'incomplete_rules'} for file in fresh['files']]
    if stripped != initial_fresh['files'] or fresh['diagnostics'] != initial_fresh['diagnostics']:
        raise ValueError('Completeness inspection changed fresh diagnoses or section predictions')
    lock_path = current / 'fresh-inputs/corpus.lock.json'
    lock = read(lock_path)
    if digest(lock_path.read_bytes()) != fresh['corpus_sha256']:
        raise ValueError('Fresh corpus binding mismatch')
    for source in lock['sources']:
        for entry in source['documents'] + source['licenses']:
            verify_blob(ROOT / 'corpus/data/blobs' / entry['git_blob'], entry)
    historical, history_rows, history_labels = labeled(current / 'history/replay.json.gz', current / 'history/labels.json')
    validate_replay(historical)
    before_history, before_history_rows, before_history_labels = labeled(base / 'history-v3/replay.json.gz', base / 'history-v3/labels.json')
    validate_replay(before_history)
    if historical['changes'] != before_history['changes']:
        # Only diagnosis indexes may change; the actual Git changes must not.
        stripped_changes = lambda value: [{key: entry for key, entry in row.items() if key not in {'introduced', 'after_diagnostics'}} for row in value['changes']]
        if stripped_changes(historical) != stripped_changes(before_history):
            raise ValueError('History inputs changed')
    freeze = read(current / 'freeze-final.json')
    if freeze['implementation'] != fingerprints():
        raise ValueError('Current implementation differs from the final freeze')
    for value in [known, replay, fresh, historical]:
        if any(value['implementation'].get(path) != sha for path, sha in freeze['implementation'].items()):
            raise ValueError('Final evidence used different implementations')
    old_sections = score_sections(read(base / 'sections-v1/sample.json'), read(base / 'sections-v1/labels.json'), known['files'])
    fresh_sections = score_sections(read(current / 'sections/sample.json'), read(current / 'sections/labels.json'), fresh['files'])
    result = {'schema_version': 1, 'script_sha256': digest(Path(__file__).read_bytes()), 'implementation': freeze['implementation'],
        'known_corpus': {'role': 'development regression; former holdout is no longer independent', 'files': len(known['files']), 'old_diagnostics': len(old_rows), 'new_diagnostics': len(known_rows),
            'old_labels': dict(Counter(row['label'] for row in old_labels.values())), 'new_labels': dict(Counter(row['label'] for row in known_labels.values())), 'existing_diagnostics_unchanged': len(non_m3_old)},
        'first_fresh_cohort': {'role': 'failed holdout for iteration one, then reused as development data', 'files': len(first['files']), 'before': len(first_rows), 'after': len(replay_rows), 'before_labels': dict(Counter(row['label'] for row in first_labels.values()))},
        'fresh_holdout': {'role': 'independent repositories for the final implementation; English-focused', 'files': len(fresh['files']), 'rules': {code: dimensions([row for row in fresh_rows.values() if row['code'] == code], fresh_labels) for code in RULES},
            'incomplete_rule_files': dict(Counter(code for file in fresh['files'] for code in file['incomplete_rules'])), 'kinds': dict(Counter(file['result']['kind']['value'] or 'unknown' for file in fresh['files'])),
            'completeness_inspection_preserved_diagnostics_and_sections': True},
        'history': {'role': 'known regression under the original fixed kind profile', 'changes': len(historical['changes']), 'before_diagnostics': len(before_history_rows), 'after_diagnostics': len(history_rows),
            'before_blocked_changes': len({row['change_id'] for row in before_history_rows.values() if before_history_labels[row['id']]['label'] == 'fp'}),
            'after_blocked_changes': len({row['change_id'] for row in history_rows.values() if history_labels[row['id']]['label'] == 'fp'}), 'rules': noise(historical['changes'], history_rows, history_labels)},
        'sections': {'known': old_sections, 'fresh': fresh_sections, 'scope': 'Stratified author-agent labels; neither sample is a population accuracy estimate.'},
        'promotion': {'rules': [], 'human_review': False, 'natural_precision_established': False, 'rule_recall_measured': False}}
    paths = [base / 'natural-v1/diagnostics.json.gz', base / 'natural-v1/labels.json', base / 'history-v3/replay.json.gz', base / 'history-v3/labels.json']
    paths += [path for root in [current, base / 'optimization-v1/fresh-holdout'] for path in root.rglob('*') if path.is_file() and path != output]
    result['inputs'] = {path.relative_to(base).as_posix(): digest(path.read_bytes()) for path in sorted(set(paths))}
    output.write_bytes(encode(result))
    print(json.dumps({key: result[key] for key in ['known_corpus', 'first_fresh_cohort']}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, default=ROOT / 'corpus/results/m3')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    run(args.base, args.output)


if __name__ == '__main__':
    main()
