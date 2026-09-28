"""Pin and evaluate an independent M3 source cohort without executing upstream code."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import gzip
import importlib.util
import json
from pathlib import Path

from evaluate_m2 import ROOT, digest, encode, fingerprints, read, verify_blob
from evaluate_m3 import RULES
from replay_m3 import build_probe, run_probe

def corpus_tools():
    spec = importlib.util.spec_from_file_location('seiso_corpus_tools', ROOT / 'corpus/corpus.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def pin(output, sources, previous_corpora=()):
    if output.exists():
        raise ValueError(f'Preserve the pinned cohort: {output}')
    previous = read(ROOT / 'corpus/corpus.lock.json')
    known = {source['repository'].lower() for source in previous['sources']}
    prior_paths = sorted(set(previous_corpora) | set((ROOT / 'corpus/results/m3').glob('**/fresh-inputs/corpus.lock.json')))
    for path in prior_paths:
        known.update(source['repository'].lower() for source in read(path)['sources'])
    if any(source['repository'].lower() in known for source in sources):
        raise ValueError('Fresh holdout repositories overlap the earlier corpus')
    module = corpus_tools()
    with ThreadPoolExecutor(max_workers=3) as pool:
        sources = list(pool.map(module.resolve_source, [source | {'split': 'holdout', 'language_focus': 'en', 'category': 'developer-documentation'} for source in sources]))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(encode({'schema_version': 1, 'selection': 'All regular Markdown files matching the declared paths; repositories selected before reading their source text or diagnostic output.',
        'previous_corpus_sha256': digest((ROOT / 'corpus/corpus.lock.json').read_bytes()), 'additional_previous_corpora': {str(path): digest(path.read_bytes()) for path in prior_paths}, 'sources': sources}))
    print(json.dumps({source['id']: len(source['documents']) for source in sources}))


def fetch(pinned, output):
    if output.exists():
        raise ValueError(f'Preserve the fetched cohort: {output}')
    lock = read(pinned)
    module = corpus_tools()
    jobs = [(source, entry) for source in lock['sources'] for entry in source['documents'] + source['licenses']]
    with ThreadPoolExecutor(max_workers=8) as pool:
        hashes = list(pool.map(lambda pair: module.fetch_blob(*pair), jobs))
    for (_, entry), sha in zip(jobs, hashes):
        entry['sha256'] = sha
    lock['pinned_selection_sha256'] = digest(pinned.read_bytes())
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(encode(lock))
    print(f'Fetched {len(jobs)} pinned documents and license records.')


def evaluate(lock_path, profile_path, output, development_replay=False):
    if output.exists():
        raise ValueError(f'Preserve the fresh evaluation: {output}')
    lock, profile = read(lock_path), read(profile_path)
    if profile.get('previous_profile_sha256') and not development_replay:
        raise ValueError('A reused kind profile requires --development-replay')
    if profile['corpus_sha256'] != digest(lock_path.read_bytes()):
        raise ValueError('Fresh kind profile is not bound to this cohort')
    documents, metadata = [], {}
    config = 'preview=true\n'
    for source in lock['sources']:
        for entry in source['documents']:
            key = f"{source['id']}/{entry['path']}"
            decision = profile['documents'][key]
            if not decision.get('reason') or decision.get('input_sha256') != entry['sha256']:
                raise ValueError(f'Missing source-bound kind review: {key}')
            if decision['kind'] != 'unknown':
                config += f'\n[[kinds]]\npath={json.dumps(key)}\nkind={json.dumps(decision["kind"])}\n'
            path = ROOT / 'corpus/data/blobs' / entry['git_blob']
            verify_blob(path, entry)
            documents.append({'id': key, 'path': key, 'source': path.read_bytes().decode('utf-8')})
            metadata[key] = (source, entry)
    if profile['documents'].keys() != metadata.keys():
        raise ValueError('Fresh kind profile has missing or extra document decisions')
    config += '\n[lint]\nselect=' + json.dumps(list(RULES)) + '\n'
    before = fingerprints()
    binary = build_probe('replay_m3')
    results, raw = run_probe(binary, {'config': config, 'documents': documents}, ROOT / 'target/m3-fresh/forward')
    _, reverse = run_probe(binary, {'config': config, 'documents': list(reversed(documents))}, ROOT / 'target/m3-fresh/reverse')
    if raw != reverse or before != fingerprints():
        raise ValueError('Fresh evaluation order or implementation changed')
    files, diagnostics = [], []
    for row in results:
        source, entry = metadata[row['id']]
        if row['source_sha256'] != entry['sha256']:
            raise ValueError('The probe did not check the original source bytes')
        files.append({'source': source['id'], 'path': entry['path'], 'sha256': entry['sha256'], 'language': row['language'], 'result': row['result'], 'incomplete_rules': row['incomplete_rules'], 'section_annotations': row['section_annotations']})
        for diagnostic in row['result']['diagnostics']:
            identity = {'source': source['id'], 'path': entry['path'], 'input_sha256': entry['sha256'], 'code': diagnostic['code'], 'span': diagnostic['byte_range']}
            diagnostics.append(identity | {'id': digest(encode(identity)), 'split': 'tuning' if development_replay else 'holdout', 'repository': source['repository'], 'commit': source['commit'], 'git_blob': entry['git_blob'], 'kind': row['result']['kind']['value'] or 'unknown', 'language': row['language'], 'diagnostic': diagnostic})
    role = 'development replay of a previously reviewed cohort; not independent validation' if development_replay else 'fresh repository holdout; English-focused; author-agent review is separate'
    report = {'schema_version': 1, 'evaluation_role': role, 'corpus_sha256': digest(lock_path.read_bytes()), 'kind_profile_sha256': digest(profile_path.read_bytes()), 'implementation': before, 'implementation_hash_format': 'sha256-lf',
        'script_sha256': digest(Path(__file__).read_bytes()), 'probe_sha256': digest(binary.read_bytes()), 'probe_source_sha256': digest((ROOT / 'examples/replay_m3.rs').read_bytes()), 'reverse_byte_identical': True, 'config': config, 'files': files, 'diagnostics': diagnostics}
    packed = gzip.compress(encode(report), mtime=0)
    output.mkdir(parents=True)
    (output / 'diagnostics.json.gz').write_bytes(packed)
    (output / 'run.json').write_bytes(encode({key: value for key, value in report.items() if key not in {'files', 'diagnostics', 'config'}} | {'report_sha256': digest(packed), 'files': len(files),
        'counts': {code: sum(row['code'] == code for row in diagnostics) for code in RULES}, 'incomplete_rule_files': dict(Counter(code for file in files for code in file['incomplete_rules'])), 'kinds': dict(Counter(file['result']['kind']['value'] or 'unknown' for file in files)), 'annotation_status': 'unlabeled'}))
    print(json.dumps({'files': len(files), 'counts': Counter(row['code'] for row in diagnostics)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['pin', 'fetch', 'evaluate'])
    parser.add_argument('--input', type=Path)
    parser.add_argument('--profile', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source', action='append', help='Public OWNER/REPO; pin selects README.md and docs/** Markdown')
    parser.add_argument('--previous-corpus', type=Path, action='append', default=[])
    parser.add_argument('--development-replay', action='store_true')
    args = parser.parse_args()
    if args.command == 'pin':
        if not args.source:
            parser.error('pin requires at least one --source OWNER/REPO')
        sources = [{'id': repo.split('/')[-1], 'repository': repo, 'include': ['README.md', 'docs/**']} for repo in args.source]
        pin(args.output, sources, args.previous_corpus)
    elif args.command == 'fetch':
        if args.input is None:
            parser.error('fetch requires --input')
        fetch(args.input, args.output)
    else:
        if args.input is None or args.profile is None:
            parser.error('evaluate requires --input and --profile')
        evaluate(args.input, args.profile, args.output, args.development_replay)


if __name__ == '__main__':
    main()
