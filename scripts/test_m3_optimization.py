import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import fresh_m3
from evaluate_m2 import digest, encode
from summarize_m3_optimization import compare_cohort, score_sections


class OptimizationEvidenceTests(unittest.TestCase):
    def test_pin_rejects_a_previously_reviewed_repository_before_network_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lock = root / 'corpus/corpus.lock.json'
            lock.parent.mkdir()
            lock.write_bytes(encode({'sources': []}))
            prior = root / 'corpus/results/m3/prior/fresh-inputs/corpus.lock.json'
            prior.parent.mkdir(parents=True)
            prior.write_bytes(encode({'sources': [{'repository': 'owner/reviewed'}]}))
            with patch.object(fresh_m3, 'ROOT', root), patch.object(fresh_m3, 'corpus_tools') as network:
                with self.assertRaisesRegex(ValueError, 'overlap'):
                    fresh_m3.pin(root / 'new.json', [{'repository': 'OWNER/REVIEWED'}])
                network.assert_not_called()

    def test_cohort_comparison_rejects_policy_and_source_drift(self):
        original = {'files': [{'source': 'project', 'path': 'doc.md', 'sha256': 'input', 'result': {'kind': {'value': 'howto'}}}]}
        compare_cohort(original, copy.deepcopy(original))
        for mutation in ['sha256', 'kind', 'missing']:
            changed = copy.deepcopy(original)
            if mutation == 'sha256':
                changed['files'][0]['sha256'] = 'changed'
            elif mutation == 'kind':
                changed['files'][0]['result']['kind']['value'] = 'reference'
            else:
                changed['files'] = []
            with self.assertRaises(ValueError):
                compare_cohort(original, changed)

    def test_section_replay_keeps_gold_labels_and_checks_input_binding(self):
        sample = {'samples': [{'id': 's', 'source': 'project', 'path': 'doc.md', 'section': 0, 'input_sha256': 'input', 'annotation_sha256': 'old', 'annotation': {'section_type': 'other'}}]}
        labels = {'labels': [{'id': 's', 'input_sha256': 'input', 'annotation_sha256': 'old', 'expected_type': 'steps'}]}
        files = [{'source': 'project', 'path': 'doc.md', 'sha256': 'input', 'section_annotations': [{'section_type': 'steps'}]}]
        result = score_sections(sample, labels, files)
        self.assertEqual((result['before_matches'], result['after_matches']), (0, 1))
        files[0]['sha256'] = 'new source'
        with self.assertRaises(ValueError):
            score_sections(sample, labels, files)

    def test_fresh_probe_receives_original_crlf_bytes_and_reports_abstention(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = '# 示例\r\n\r\nContent.\r\n'.encode()
            import hashlib
            blob = hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()
            blob_path = root / 'corpus/data/blobs' / blob
            blob_path.parent.mkdir(parents=True)
            blob_path.write_bytes(raw)
            (root / 'examples').mkdir()
            (root / 'examples/replay_m3.rs').write_text('probe', encoding='utf-8')
            binary = root / 'probe.exe'
            binary.write_bytes(b'probe')
            lock = {'sources': [{'id': 'project', 'repository': 'owner/project', 'commit': 'a' * 40, 'documents': [{'path': 'README.md', 'git_blob': blob, 'bytes': len(raw), 'sha256': digest(raw)}]}]}
            lock_path, profile_path = root / 'lock.json', root / 'kinds.json'
            lock_path.write_bytes(encode(lock))
            profile_path.write_bytes(encode({'corpus_sha256': digest(lock_path.read_bytes()), 'implementation': {'engine': 'fixed'}, 'documents': {'project/README.md': {'kind': 'readme', 'reason': 'Introduction', 'input_sha256': digest(raw)}}}))
            def probe(binary, batch, work):
                self.assertEqual(batch['documents'][0]['source'].encode(), raw)
                rows = [{'id': 'project/README.md', 'source_sha256': digest(raw), 'language': 'zh', 'result': {'kind': {'value': 'readme'}, 'diagnostics': []}, 'section_annotations': [], 'incomplete_rules': ['EVD001']}]
                return rows, encode(rows)
            with patch.object(fresh_m3, 'ROOT', root), patch.object(fresh_m3, 'fingerprints', return_value={'engine': 'fixed'}), patch.object(fresh_m3, 'build_probe', return_value=binary), patch.object(fresh_m3, 'run_probe', side_effect=probe):
                fresh_m3.evaluate(lock_path, profile_path, root / 'result')
            result = json.loads((root / 'result/run.json').read_bytes())
            self.assertEqual(result['incomplete_rule_files'], {'EVD001': 1})
            self.assertEqual(result['counts']['EVD001'], 0)


if __name__ == '__main__':
    unittest.main()
