import copy
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import review_m2_links as oracle


class OracleCohortInputTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.corpus = self.root / 'corpus'
        self.corpus.mkdir()
        self.shared = self.corpus / 'data/blobs'
        self.shared.mkdir(parents=True)
        self.custom_inventory = self.root / 'cohort/snapshots'
        self.custom_inventory.mkdir(parents=True)
        self.original = b'[guide](docs/page.md#missing)\n'
        self.target = b'# Existing\n'
        self.documents = []
        entries = []
        for name, raw in [('README.md', self.original), ('docs/page.md', self.target)]:
            blob = hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()
            record = {'path': name, 'git_blob': blob, 'bytes': len(raw), 'sha256': oracle.digest(raw)}
            self.documents.append(record)
            self.shared.joinpath(blob).write_bytes(raw)
            entries.append({'path': name, 'mode': '100644', 'type': 'blob', 'sha': blob})
        identity = {'id': 'sample', 'repository': 'owner/sample', 'commit': 'a' * 40, 'tree': 'b' * 40}
        self.lock = {'schema_version': 1, 'sources': [identity | {'split': 'holdout', 'documents': self.documents}]}
        self.lock_path = self.root / 'cohort/corpus.lock.json'
        self.lock_path.write_bytes(oracle.encode(self.lock))
        archive = gzip.compress(oracle.encode(identity | {'schema_version': 1, 'entries': entries}), mtime=0)
        (self.custom_inventory / 'sample.json.gz').write_bytes(archive)
        self.inventory = {'schema_version': 1, 'corpus_lock_sha256': oracle.digest(self.lock_path.read_bytes()),
            'sources': [identity | {'archive': 'sample.json.gz', 'sha256': oracle.digest(archive), 'entries': 2}]}
        self.inventory_path = self.custom_inventory / 'inventory.lock.json'
        self.inventory_path.write_bytes(oracle.encode(self.inventory))
        self.slugger = self.root / 'slugger/index.js'
        self.slugger.parent.mkdir()
        self.slugger.write_bytes(b'locked slugger index')
        self.slugger.with_name('regex.js').write_bytes(b'locked slugger regex')
        self.slugger.with_name('package.json').write_bytes(b'{"version":"2.0.0"}')
        span = {'start': 0, 'end': len(self.original) - 1}
        destination = 'docs/page.md#missing'
        start = self.original.index(destination.encode())
        link = {'span': span, 'destination': destination,
            'destination_span': {'start': start, 'end': start + len(destination)}, 'reference': None}
        self.candidate = self.root / 'candidate.json'
        self.candidate.write_bytes(oracle.encode([{'source': 'sample', 'path': 'README.md',
            'sha256': self.documents[0]['sha256'], 'links': [link],
            'result': {'diagnostics': [{'code': 'LNK002', 'byte_range': span, 'location': {'row': 1}}]}}]))
        self.root_patch = patch.object(oracle, 'CORPUS', self.corpus)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def facts(self, raw):
        if raw == self.original:
            return {'headings': [], 'html_anchors': [], 'destinations': ['docs/page.md#missing']}
        self.assertEqual(raw, self.target)
        return {'headings': [{'text': 'Existing', 'line': 1}], 'html_anchors': [], 'destinations': []}

    def review(self, **kwargs):
        result = subprocess.CompletedProcess([], 0, stdout=b'[[], ["existing"]]')
        with patch.object(oracle, 'content_facts', side_effect=self.facts), \
             patch.object(oracle.subprocess, 'run', return_value=result), \
             patch.object(oracle.importlib.metadata, 'version', return_value='4.0.0'):
            return oracle.review(self.candidate, self.slugger, **kwargs)

    def test_optional_inputs_use_custom_archive_directory_and_shared_original_blobs(self):
        result = self.review(corpus_lock=self.lock_path, inventory=self.inventory_path)
        self.assertEqual(result['corpus_sha256'], oracle.digest(self.lock_path.read_bytes()))
        self.assertEqual(result['inventory_sha256'], oracle.digest(self.inventory_path.read_bytes()))
        self.assertEqual(result['labels'][0]['target_sha256'], self.documents[1]['sha256'])
        self.assertTrue(result['labels'][0]['source_verified'])
        self.assertEqual(result['labels'][0]['label'], 'tp')
        self.assertIn('GitHub-only', result['scope'])

    def test_defaults_and_explicit_main_inputs_produce_identical_oracle_results(self):
        (self.corpus / 'corpus.lock.json').write_bytes(self.lock_path.read_bytes())
        main_inventory = self.corpus / 'inventory'
        main_inventory.mkdir()
        for path in self.custom_inventory.iterdir():
            (main_inventory / path.name).write_bytes(path.read_bytes())
        default = self.review()
        explicit = self.review(corpus_lock=self.corpus / 'corpus.lock.json',
                               inventory=main_inventory / 'inventory.lock.json')
        self.assertEqual(oracle.encode(default), oracle.encode(explicit))

    def test_custom_inventory_rejects_wrong_lock_source_identity_archive_and_entry_count(self):
        for change in ['lock', 'duplicate', 'identity', 'archive', 'entries', 'checksum']:
            inventory = copy.deepcopy(self.inventory)
            if change == 'lock':
                inventory['corpus_lock_sha256'] = '0' * 64
            elif change == 'duplicate':
                inventory['sources'].append(inventory['sources'][0])
            elif change == 'identity':
                inventory['sources'][0]['commit'] = 'c' * 40
            elif change == 'archive':
                inventory['sources'][0]['archive'] = '../sample.json.gz'
            elif change == 'entries':
                inventory['sources'][0]['entries'] = 3
            else:
                inventory['sources'][0]['sha256'] = '0' * 64
            self.inventory_path.write_bytes(oracle.encode(inventory))
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.review(corpus_lock=self.lock_path, inventory=self.inventory_path)


if __name__ == '__main__':
    unittest.main()
