import copy
import gzip
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import fresh_m3
from evaluate_m2 import digest, encode, source_digest, verify_blob
from summarize_m3_optimization import compare_cohort, run, score_sections
from verify_m3 import same_results


class OptimizationEvidenceTests(unittest.TestCase):
    def test_source_identity_ignores_checkout_newlines_but_corpus_identity_does_not(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'source.rs'
            lf = b'fn main() {\n}\n'
            path.write_bytes(lf)
            expected = source_digest(path)
            path.write_bytes(lf.replace(b'\n', b'\r\n'))
            self.assertEqual(source_digest(path), expected)
            with self.assertRaises(ValueError):
                verify_blob(path, {'bytes': len(lf), 'sha256': digest(lf)})
            path.write_bytes(b'fn main() { panic!(); }\n')
            self.assertNotEqual(source_digest(path), expected)

    def test_replay_accepts_added_completeness_metadata_but_detects_behavior_changes(self):
        baseline = [{'id': 'doc', 'source_sha256': 'input', 'language': 'en',
                     'result': {'diagnostics': []}, 'section_annotations': []}]
        current = [baseline[0] | {'incomplete_rules': ['RAT001']}]
        self.assertTrue(same_results(current, baseline))
        self.assertTrue(same_results(current, copy.deepcopy(current)))
        for changed in [[], [current[0] | {'source_sha256': 'other'}],
                        [current[0] | {'result': {'diagnostics': ['new']}}],
                        [current[0] | {'section_annotations': ['new']}]]:
            self.assertFalse(same_results(changed, baseline))
        self.assertFalse(same_results([current[0] | {'incomplete_rules': []}], current))
        self.assertFalse(same_results(baseline, current))

    def test_summary_uses_manifest_inputs_without_current_sources_or_directory_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            packed = gzip.compress(encode({'implementation': {'old.rs': 'historical'},
                'files': [{'source': 'project', 'path': 'doc.md', 'sha256': 'input', 'result': {'kind': {'value': 'howto'}}}],
                'diagnostics': []}), mtime=0)
            (root / 'sample.gz').write_bytes(packed)
            (root / 'review.json').write_bytes(encode({'schema_version': 1, 'report_sha256': digest(packed), 'reviewer_kind': 'author_agent', 'labels': []}))
            case = {'report': 'sample.gz', 'labels': 'review.json'}
            manifest = root / 'inputs.json'
            manifest.write_bytes(encode({'comparisons': {'custom': {'before': case, 'after': case, 'preserve_existing_rules': True}}}))
            run(manifest, root / 'first.json')
            (root / 'unrelated.json').write_text('unrelated output', encoding='utf-8')
            run(manifest, root / 'second.json')
            self.assertEqual((root / 'first.json').read_bytes(), (root / 'second.json').read_bytes())
            result = json.loads((root / 'first.json').read_bytes())
            self.assertEqual(set(result['inputs']), {'sample.gz', 'review.json'})
            self.assertEqual(result['comparisons']['custom']['files'], 1)

    def test_pin_requires_explicit_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([sys.executable, str(Path(fresh_m3.__file__)), 'pin', '--output', str(Path(directory) / 'selection.json')], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn(b'pin requires at least one --source', result.stderr)

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
            profile_path.write_bytes(encode({'corpus_sha256': digest(lock_path.read_bytes()), 'implementation': {'engine': 'older-profile'}, 'documents': {'project/README.md': {'kind': 'readme', 'reason': 'Introduction', 'input_sha256': digest(raw)}}}))
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
