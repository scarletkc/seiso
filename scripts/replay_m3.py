"""Freeze real Git Markdown changes and newly introduced heuristic diagnostics."""

import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
import shutil
import subprocess

from evaluate_m2 import ROOT, digest, encode, fingerprints, source_digest
from evaluate_m3 import RULES

POLICY = '''preview=true
[[kinds]]
path='README.md'
kind='readme'
[[kinds]]
path='CONTRIBUTING.md'
kind='howto'
[[kinds]]
path='docs/**'
kind='reference'
[[kinds]]
path='docs/release-notes/**'
kind='changelog'
[lint]
select=''' + json.dumps(list(RULES)) + '\n'


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def collect(revision):
    revision = git('rev-parse', '--verify', revision + '^{commit}').decode().strip()
    commits = git('rev-list', '--first-parent', '--reverse', revision).decode().splitlines()
    changes, documents = [], {}
    for commit in commits:
        parents = git('rev-list', '--parents', '-n', '1', commit).decode().split()[1:]
        parent = parents[0] if parents else None
        fields = git('diff-tree', '--root', '--no-commit-id', '--no-renames', '--name-status', '-r', '-z', *([parent] if parent else []), commit).decode().strip('\0').split('\0')
        if fields == ['']:
            continue
        for status, path in zip(fields[::2], fields[1::2]):
            if not path.lower().endswith(('.md', '.markdown')) or status not in {'A', 'M', 'D'}:
                continue
            pair = {}
            for side, ref, absent in [('before', parent, status == 'A'), ('after', commit, status == 'D')]:
                raw = b'' if absent or ref is None else git('show', f'{ref}:{path}')
                identity = digest(encode({'path': path, 'sha256': digest(raw)}))
                documents[identity] = {'id': identity, 'path': path, 'source': raw.decode('utf-8'), 'source_sha256': digest(raw)}
                pair[side] = identity
            changes.append({'id': digest(encode({'commit': commit, 'path': path})), 'commit': commit, 'parent': parent, 'path': path, 'status': status, **pair})
    return revision, changes, documents


def signature(diagnostic, source):
    span = diagnostic['byte_range']
    excerpt = source.encode()[span['start']:span['end']].decode()
    # Source position changes alone do not make an unchanged warning new.
    return diagnostic['code'], ' '.join(excerpt.split()), diagnostic['message']


def introduced(before, after, before_source, after_source):
    remaining = Counter(signature(row, before_source) for row in before)
    added = []
    for row in after:
        key = signature(row, after_source)
        if remaining[key]:
            remaining[key] -= 1
        else:
            added.append(row)
    return added


def build_probe(name):
    cargo = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo.exe')
    build = subprocess.run([cargo, 'build', '--release', '--locked', '--example', name, '--message-format=json'], cwd=ROOT, capture_output=True, check=True)
    return next(Path(event['executable']) for line in build.stdout.splitlines() if (event := json.loads(line)).get('reason') == 'compiler-artifact' and event.get('target', {}).get('name') == name and event.get('executable'))


def run_probe(binary, inputs, work):
    work.mkdir(parents=True, exist_ok=True)
    input_path, output_path = work / 'input.json', work / 'output.json'
    input_path.write_bytes(encode(inputs))
    subprocess.run([str(binary), str(input_path), str(output_path)], check=True)
    return json.loads(output_path.read_bytes()), output_path.read_bytes()


def run(output, revision):
    if output.exists():
        raise ValueError(f'Preserve the existing replay: {output}')
    revision, changes, documents = collect(revision)
    before = fingerprints() | {Path(__file__).relative_to(ROOT).as_posix(): source_digest(Path(__file__)), 'examples/replay_m3.rs': source_digest(ROOT / 'examples/replay_m3.rs')}
    binary = build_probe('replay_m3')
    batch = {'config': POLICY, 'documents': list(documents.values())}
    results, raw = run_probe(binary, batch, ROOT / 'target/m3-history/forward')
    _, reverse = run_probe(binary, batch | {'documents': list(reversed(batch['documents']))}, ROOT / 'target/m3-history/reverse')
    if raw != reverse or any(source_digest(ROOT / path) != sha for path, sha in before.items()):
        raise ValueError('Replay implementation or order equivalence changed')
    by_id = {row['id']: row for row in results}
    diagnoses = []
    for change in changes:
        old, new = (by_id[change[side]] for side in ['before', 'after'])
        added = introduced(old['result']['diagnostics'], new['result']['diagnostics'], documents[change['before']]['source'], documents[change['after']]['source'])
        change['introduced'] = []
        change['after_diagnostics'] = []
        remaining_new = Counter(encode(diagnostic) for diagnostic in added)
        for index, diagnostic in enumerate(new['result']['diagnostics']):
            identity = digest(encode({'change': change['id'], 'diagnostic': diagnostic, 'ordinal': index}))
            change['after_diagnostics'].append(identity)
            key = encode(diagnostic)
            is_new = remaining_new[key] > 0
            if is_new:
                change['introduced'].append(identity)
                remaining_new[key] -= 1
            diagnoses.append({'id': identity, 'change_id': change['id'], 'code': diagnostic['code'], 'path': change['path'], 'input_sha256': new['source_sha256'], 'diagnostic': diagnostic, 'document_id': change['after'], 'introduced': is_new})
    report = {'schema_version': 1, 'revision': revision, 'repository': 'https://github.com/scarletkc/seiso', 'license': 'MIT', 'selection': 'All first-parent A/M/D Markdown path changes through the pinned revision; renames are delete plus add.',
        'policy': POLICY, 'policy_sha256': digest(POLICY.encode()), 'policy_scope': 'One fixed current kind profile on both sides isolates content changes; original per-revision configuration is not replayed.',
        'implementation': before, 'implementation_hash_format': 'sha256-lf', 'probe_sha256': digest(binary.read_bytes()), 'reverse_byte_identical': True,
        'documents': list(documents.values()), 'results': results, 'changes': changes, 'diagnostics': diagnoses}
    packed = gzip.compress(encode(report), mtime=0)
    output.mkdir(parents=True)
    (output / 'replay.json.gz').write_bytes(packed)
    (output / 'run.json').write_bytes(encode({'schema_version': 1, 'report_sha256': digest(packed), 'revision': revision, 'changes': len(changes), 'documents': len(documents), 'after_diagnostics': dict(sorted(Counter(row['code'] for row in diagnoses).items())), 'introduced': dict(sorted(Counter(row['code'] for row in diagnoses if row['introduced']).items())), 'annotation_status': 'unlabeled; all post-change diagnoses count toward blocking, including persistent warnings; clean changes remain in the denominator'}))
    print(json.dumps({'changes': len(changes), 'after_diagnostics': Counter(row['code'] for row in diagnoses), 'introduced': Counter(row['code'] for row in diagnoses if row['introduced'])}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    run(args.output, args.revision)


if __name__ == '__main__':
    main()
