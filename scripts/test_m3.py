import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

import evaluate_m2
import evaluate_m3
import replay_m3
import summarize_m3
from summarize_m2 import validate_labels


class M3Tests(unittest.TestCase):
    def test_replay_ignores_position_only_changes_and_counts_duplicates(self):
        old = {'code': 'STL002', 'message': 'state', 'byte_range': {'start': 0, 'end': 4}}
        shifted = old | {'byte_range': {'start': 2, 'end': 6}}
        self.assertEqual(replay_m3.introduced([old], [shifted], 'live', '\n\nlive'), [])
        self.assertEqual(replay_m3.introduced([old], [shifted, shifted], 'live', '\n\nlive'), [shifted])
        self.assertEqual(replay_m3.introduced([old], [shifted | {'code': 'EVD001'}], 'live', '\n\nlive')[0]['code'], 'EVD001')

    def test_noise_counts_changes_once_and_keeps_clean_changes(self):
        changes = [{'id': str(i)} for i in range(100)]
        rows = {str(i): {'id': str(i), 'code': 'EVD001', 'change_id': str(i // 2), 'introduced': i == 0} for i in range(5)}
        labels = {str(i): {'label': 'fp' if i < 3 else 'uncertain'} for i in range(5)}
        result = summarize_m3.noise(changes, rows, labels)
        self.assertEqual(set(result), set(evaluate_m3.RULES))
        self.assertEqual(result['EVD001']['incorrect_blocks_per_100_changes'], 2)
        self.assertEqual(result['EVD001']['conservative_blocks_per_100_changes'], 3)
        self.assertEqual(result['EVD001']['changes_with_new_diagnostics'], 1)
        self.assertEqual(result['EVD001']['changes_with_diagnostics'], 3)
        self.assertEqual(result['STL002']['changes'], 100)
        self.assertEqual(result['STL002']['incorrect_blocks_per_100_changes'], 0)
        self.assertIsNone(summarize_m3.noise([], {}, {})['STL002']['incorrect_blocks_per_100_changes'])
        with self.assertRaises(ValueError):
            summarize_m3.noise([{'id': '0'}, {'id': '0'}], {}, {})

    def test_labels_reject_stale_hashes_unknowns_missing_and_duplicate_judgments(self):
        row = {'id': 'id', 'code': 'STL002', 'input_sha256': 'input', 'diagnostic': {'code': 'STL002'}}
        label = {key: row[key] for key in ['id', 'code', 'input_sha256']} | {'diagnostic_sha256': evaluate_m2.digest(evaluate_m2.encode(row['diagnostic'])), 'label': 'uncertain', 'reason': 'Deployment context is ambiguous.'}
        bundle = {'schema_version': 1, 'report_sha256': 'report', 'reviewer_kind': 'author_agent', 'labels': [label]}
        validate_labels([('labels.json', bundle)], {'id': row}, 'report')
        mutations = [bundle | {'report_sha256': 'old'}, bundle | {'labels': []}, bundle | {'labels': [label, label]}]
        for key, value in [('input_sha256', 'old'), ('diagnostic_sha256', 'old'), ('label', 'unreviewed'), ('reason', ''), ('id', 'unknown')]:
            mutations.append(bundle | {'labels': [label | {key: value}]})
        for mutated in mutations:
            with self.assertRaises(ValueError):
                validate_labels([('labels.json', mutated)], {'id': row}, 'report')

    def test_reverse_keeps_section_inspection_enabled(self):
        batch = {'sections': True, 'sources': []}
        self.assertEqual(evaluate_m2.reverse_inputs(batch), batch)

    def test_persistent_diagnostics_cannot_be_dropped_from_usage_noise(self):
        path = evaluate_m2.ROOT / 'corpus/results/m3/history-v3/replay.json.gz'
        replay = json.loads(gzip.decompress(path.read_bytes()))
        rows = summarize_m3.validate_replay(replay)
        self.assertGreater(len(rows), sum(row['introduced'] for row in rows.values()))
        changed = copy.deepcopy(replay)
        change = next(change for change in changed['changes'] if change['after_diagnostics'] and not change['introduced'])
        change['after_diagnostics'] = []
        with self.assertRaisesRegex(ValueError, 'omitted or changed'):
            summarize_m3.validate_replay(changed)


if __name__ == '__main__':
    unittest.main()
