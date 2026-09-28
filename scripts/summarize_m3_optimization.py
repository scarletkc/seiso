"""Bind M3 optimization evidence while separating reused data from fresh evaluation."""

import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from evaluate_m2 import digest, encode, read
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


def run(manifest_path, output):
    if output.exists():
        raise ValueError(f'Preserve the existing summary: {output}')
    manifest = read(manifest_path)
    inputs = {}

    def resolve(relative):
        path = manifest_path.parent / relative
        inputs[relative] = digest(path.read_bytes())
        return path

    def load_case(case):
        return labeled(resolve(case['report']), resolve(case['labels']))

    result = {'schema_version': 2, 'manifest_sha256': digest(manifest_path.read_bytes()),
              'comparisons': {}, 'cohorts': {}, 'histories': {}, 'sections': {}}
    for name, case in manifest.get('comparisons', {}).items():
        before, before_rows, before_labels = load_case(case['before'])
        after, after_rows, after_labels = load_case(case['after'])
        compare_cohort(before, after)
        summary = {'files': len(after['files']), 'before_diagnostics': len(before_rows), 'after_diagnostics': len(after_rows),
                   'before_labels': dict(Counter(row['label'] for row in before_labels.values())),
                   'after_labels': dict(Counter(row['label'] for row in after_labels.values()))}
        if case.get('preserve_existing_rules'):
            old = {row['id']: row['diagnostic'] for row in before['diagnostics'] if row['code'] not in RULES}
            new = {row['id']: row['diagnostic'] for row in after['diagnostics'] if row['code'] not in RULES}
            if old != new:
                raise ValueError(f'{name}: existing-rule diagnostics changed')
            summary['existing_diagnostics_unchanged'] = len(old)
        result['comparisons'][name] = summary

    for name, case in manifest.get('cohorts', {}).items():
        cohort, rows, labels = load_case(case)
        if digest(resolve(case['corpus']).read_bytes()) != cohort['corpus_sha256']:
            raise ValueError(f'{name}: corpus binding mismatch')
        result['cohorts'][name] = {
            'role': cohort['evaluation_role'], 'files': len(cohort['files']),
            'rules': {code: dimensions([row for row in rows.values() if row['code'] == code], labels) for code in RULES},
            'incomplete_rule_files': dict(Counter(code for file in cohort['files'] for code in file.get('incomplete_rules', []))),
            'kinds': dict(Counter(file['result']['kind']['value'] or 'unknown' for file in cohort['files']))}

    for name, case in manifest.get('histories', {}).items():
        before, before_rows, before_labels = load_case(case['before'])
        after, after_rows, after_labels = load_case(case['after'])
        validate_replay(before)
        validate_replay(after)
        def changes(value):
            return [{key: entry for key, entry in row.items() if key not in {'introduced', 'after_diagnostics'}} for row in value['changes']]
        if changes(before) != changes(after) or before['policy'] != after['policy']:
            raise ValueError(f'{name}: history inputs or policy changed')
        result['histories'][name] = {
            'changes': len(after['changes']), 'before_diagnostics': len(before_rows), 'after_diagnostics': len(after_rows),
            'before_blocked_changes': len({row['change_id'] for row in before_rows.values() if before_labels[row['id']]['label'] == 'fp'}),
            'after_blocked_changes': len({row['change_id'] for row in after_rows.values() if after_labels[row['id']]['label'] == 'fp'}),
            'rules': noise(after['changes'], after_rows, after_labels)}

    for name, case in manifest.get('sections', {}).items():
        cohort = report(resolve(case['report']))
        result['sections'][name] = score_sections(read(resolve(case['sample'])), read(resolve(case['labels'])), cohort['files'])
    result['inputs'] = inputs
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(encode(result))
    print(json.dumps(result['comparisons']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    run(args.manifest, args.output)


if __name__ == '__main__':
    main()
