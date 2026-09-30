import collections
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import traceback

ROOT = Path('/workspace/seiso')
F = ROOT / 'corpus/results/lnk002/fresh-v1'
B = F / 'batch-1'
A = B / 'annotations'
checks, facts, hashes, errors = [], {}, {}, []
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT))

def sha(raw):
    return hashlib.sha256(raw).hexdigest()

def read(path):
    return json.loads(path.read_bytes())

def enc(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()

def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)

def track(path):
    hashes[path.relative_to(ROOT).as_posix()] = sha(path.read_bytes())

def passed(name):
    checks.append({'check': name, 'passed': True})

def score(rows, labels):
    count = collections.Counter(labels[r['id']]['label'] for r in rows)
    n = len(rows)
    known = count['tp'] + count['fp']
    agreements = sum(labels[r['id']]['author_review']['label'] == labels[r['id']]['independent_review']['label'] for r in rows)
    return {'samples': n, 'tp': count['tp'], 'fp': count['fp'], 'uncertain': count['uncertain'],
            'precision': count['tp']/known if known else None,
            'conservative_precision': count['tp']/n if n else None,
            'agreements': agreements, 'agreement_rate': agreements/n if n else None}

def percent(value):
    return 'unavailable' if value is None else f'{value*100:.1f}%'

def table_rows(text, heading):
    start = text.index(heading)
    lines = text[start:].splitlines()
    result = []
    for line in lines[2:]:
        if not line.startswith('|'):
            break
        result.append([x.strip() for x in line.strip('|').split('|')])
    return result

try:
    manifest = read(F/'manifest.json')
    summary = read(F/'summary.json')
    findings = read(F/'findings.json')
    decision = read(F/'decision.json')
    freeze = read(F/'freeze.json')
    spec = read(F/'sources.json')
    lock = read(B/'corpus.lock.json')
    report_packed = (B/'run/diagnostics.json.gz').read_bytes()
    report = json.loads(gzip.decompress(report_packed))
    run = read(B/'run/run.json')
    bound = read(A/'labels-links.json')
    labels = {x['id']: x for x in bound['labels']}
    rows = [x for x in report['diagnostics'] if x['code'] == 'LNK002']
    assert len(rows) == len(labels) == len(bound['labels']) == 451
    assert {x['id'] for x in rows} == set(labels)
    assert manifest['batches'] == [{'batch':1,'corpus_lock':'batch-1/corpus.lock.json','labels':'batch-1/annotations/labels-links.json','report':'batch-1/run/diagnostics.json.gz'}]
    assert [p.name for p in F.glob('batch-*') if p.is_dir()] == ['batch-1']
    assert len(spec['sources']) == 25
    assert dict(collections.Counter(x['batch'] for x in spec['sources'])) == {1:10,2:5,3:5,4:5}
    assert (F/'sources.json').read_bytes() == git('show','6d2ad1431a17ec07beab8bba3c559a99553f78b5:corpus/results/lnk002/fresh-v1/sources.json')
    assert len(lock['sources']) == 10 and sum(len(x['documents']) for x in lock['sources']) == 2230
    assert not lock['skipped']
    assert sha(report_packed) == bound['report_sha256'] == run['report_sha256'] == '5d674b83996528dfd4702bbd7c15732cc82be29a889e4edf7ab20eb07cd04cac'
    assert sha((B/'corpus.lock.json').read_bytes()) == report['corpus_sha256'] == bound['corpus_sha256']
    expected_inputs = {x:sha((F/x).read_bytes()) for x in ['batch-1/annotations/labels-links.json','batch-1/corpus.lock.json','batch-1/run/diagnostics.json.gz']}
    assert summary['inputs'] == expected_inputs
    assert summary['manifest_sha256'] == sha((F/'manifest.json').read_bytes())
    assert decision['summary_sha256'] == sha((F/'summary.json').read_bytes())
    passed('exact manifest, first-batch stopping, precommitted four-batch specification and report/label/input bindings')

    overall = score(rows, labels)
    assert overall == summary['overall']
    assert overall == {'samples':451,'tp':96,'fp':306,'uncertain':49,'precision':96/402,'conservative_precision':96/451,'agreements':443,'agreement_rate':443/451}
    assert summary['by_batch'] == {'1':dict(overall,skipped=[])}
    sources = {s['id']:s for s in lock['sources']}
    expected_source = {sid:{'repository':s['repository'],'batch':1,**score([r for r in rows if r['source']==sid],labels)} for sid,s in sources.items()}
    expected_language = {lang:score([r for r in rows if r['language']==lang],labels) for lang in ['en','ja','zh']}
    assert summary['by_source'] == expected_source
    assert summary['by_language'] == expected_language
    assert summary['kind_breakdown'] == {'status':'unavailable','reason':'Document responsibilities were not reviewed for this link-rule holdout.'}
    assert summary['gate'] == {'all_diagnoses_reviewed_by_two_agents':True,'minimum_conservative_precision':0.95,'minimum_samples':100,'numeric_threshold_met':False,'passes':False,'promotes_rule':False,'source_diversity_review_met':False,'sources_over_half':['vitest']}
    assert summary['source_diversity_review'] == manifest['source_diversity_review']
    assert manifest['source_diversity_review']['reviewed'] is True and manifest['source_diversity_review']['owner_accepted'] is None
    assert summary['review_provenance']['1']['human_reviewers'] == bound['human_reviewers'] == decision['human_reviewers'] == 0
    assert decision['decision'] == 'retain_preview'
    for key in ['conservative_precision_gate_met','gate_passes','source_diversity_gate_met','main_changed','product_code_changed_after_freeze','promotion_performed','pull_requests_merged']:
        assert decision[key] is False, key
    for key in ['dual_review_gate_met','sample_size_gate_met','future_fix_requires_another_fresh_cohort','future_main_integration_replay_required']:
        assert decision[key] is True, key
    assert decision['source_diversity_owner_accepted'] is None
    facts['independently_recomputed_overall'] = overall
    facts['per_source'] = expected_source
    facts['per_language'] = expected_language
    passed('independent arithmetic for overall/batch/source/language/agreement and honest failed gate with pending diversity')

    from scripts.evaluation import fresh_links
    assert sha(Path(fresh_links.__file__).read_bytes()) == freeze['implementation']['scripts/evaluation/fresh_links.py'] == summary['summary_script_sha256']
    replay_path = Path('/tmp/seiso-stage7-independent-summary.json')
    if replay_path.exists(): replay_path.unlink()
    fresh_links.summarize(F/'manifest.json',replay_path)
    assert replay_path.read_bytes() == (F/'summary.json').read_bytes()
    facts['frozen_summary_replay_byte_identical'] = True
    facts['summary_replay_path'] = str(replay_path)
    passed('frozen summarizer replay byte-identical to recorded summary after independent recomputation')

    s6 = read(B/'stage-6-independent-review.json')
    assert s6['pass'] is True and not s6['errors']
    for name,digest in s6['hashes'].items():
        if name.startswith(('annotations/','stage-6-checkpoint','run/')):
            assert sha((B/name).read_bytes()) == digest, name
    assert sha((A/'verify-stage6-bundle.py').read_bytes()) == s6['verification_script_sha256'] == '5618e92335fddb61eaa05a66fca0ec1e615507fd3c37cb30d7e962b4c1df47b2'
    author_raw = gzip.decompress((A/'author-labels.json.gz').read_bytes())
    independent_raw = gzip.decompress((A/'independent-labels.json.gz').read_bytes())
    author_seal,independent_seal = read(B/'author-annotation-seal.json'),read(B/'independent-annotation-seal.json')
    assert sha(author_raw) == author_seal['labels_sha256'] == s6['hashes']['original_uncompressed_author_labels'] == '4aa71a2ca33d84281d5212797f5a864a17668128254a7ae7930d65b85aa1879c'
    assert sha(independent_raw) == independent_seal['labels_sha256'] == s6['hashes']['original_uncompressed_independent_labels'] == '19f7eddd358f311910b0504485d10219279a1219057d79c4431dd3b63541cc0d'
    assert author_seal['independent_annotation_started'] is False and independent_seal['author_material_accessed'] is False
    assert independent_seal['original_annotation_sealed_before_reconciliation'] is True
    assert datetime.datetime.fromisoformat(author_seal['sealed_at']) < datetime.datetime.fromisoformat(independent_seal['sealed_at_utc'])
    author = {x['id']:x for x in json.loads(author_raw)['labels']}
    independent = {x['id']:x for x in json.loads(independent_raw)['labels']}
    nf = ['label','reason','renderer','evidence','reviewer','agent_context_reviewed']
    difference = []
    for row in rows:
        final = labels[row['id']]
        for key in ['id','code','source','path','span','input_sha256','related_inputs','split']:
            assert final[key] == row[key]
        assert final['diagnostic_sha256'] == sha(enc(row['diagnostic']))
        assert final['author_review'] == {k:author[row['id']][k] for k in nf}
        assert final['independent_review'] == {k:independent[row['id']][k] for k in nf}
        assert final['author_review']['agent_context_reviewed'] is final['independent_review']['agent_context_reviewed'] is True
        assert final['author_review']['reviewer'] != final['independent_review']['reviewer']
        if final['author_review']['label'] != final['independent_review']['label']:difference.append(row['id'])
    proposal = read(A/'independent-reconciliation-proposal.json')
    reconciliation = read(A/'reconciliation.json')
    proposed = {x['id']:x for x in proposal['proposals']}
    resolved = {x['id']:x for x in reconciliation['resolutions']}
    assert len(difference) == 8 and set(difference) == set(proposed) == set(resolved)
    assert {p['review_index']:p['proposed_resolved_label'] for p in proposed.values()} == {0:'fp',45:'uncertain',188:'fp',301:'fp',302:'fp',317:'fp',378:'fp',379:'fp'}
    for id,p in proposed.items():
        assert labels[id]['label'] == p['proposed_resolved_label'] == resolved[id]['resolved_label']
        for key in ['reason','renderer']:assert labels[id][key] == p[key]
        assert resolved[id]['author_original_unchanged'] is resolved[id]['independent_original_unchanged'] is True
    facts['originals_unchanged_and_blind_protocol_preserved'] = True
    facts['original_disagreements'] = 8
    facts['human_reviewers'] = 0
    passed('unchanged complete stage6 hash-chain, sealed originals, exact nested reviews and all eight resolutions')

    fs = findings['findings']
    expected_findings = {id:lab for id,lab in labels.items() if lab['label'] in ['fp','uncertain']}
    assert len(fs) == len(expected_findings) == 355
    assert {x['id'] for x in fs} == set(expected_findings)
    assert len({x['id'] for x in fs}) == len(fs)
    for finding in fs:
        assert finding == {k:expected_findings[finding['id']][k] for k in finding}
        assert {'id','source','path','target','fragment','label','category','reason','renderer','evidence','author_review','independent_review','review_index'} == set(finding)
    assert findings['labels_sha256'] == sha((A/'labels-links.json').read_bytes())
    assert findings['report_sha256'] == sha(report_packed)
    category = collections.Counter((x['label'],x['category']) for x in fs)
    assert sum(n for (label,_),n in category.items() if label=='fp') == 306
    assert sum(n for (label,_),n in category.items() if label=='uncertain') == 49
    facts['findings_total'] = len(fs)
    facts['findings_categories'] = {label+':'+cause:n for (label,cause),n in sorted(category.items())}
    passed('all306FP and49uncertain findings unique, complete and exact bound-label projections')

    record_path = ROOT/'docs/evaluation/lnk002-2026-09-30.md'
    procedure_path = ROOT/'corpus/docs/evaluation.md'
    record,procedure = record_path.read_text(),procedure_path.read_text()
    rt = table_rows(record,'| Batch/source | Samples |')
    expected_table = [['Batch 1 / total',str(overall['samples']),str(overall['tp']),str(overall['fp']),str(overall['uncertain']),percent(overall['precision']),percent(overall['conservative_precision'])]]
    expected_table += [[sid,str(m['samples']),str(m['tp']),str(m['fp']),str(m['uncertain']),percent(m['precision']),percent(m['conservative_precision'])] for sid,m in [(s['id'],expected_source[s['id']]) for s in lock['sources']]]
    assert rt == expected_table
    ct = table_rows(record,'| Outcome/cause | Count |')
    assert {(x[0].replace('`','').split(': ',1)[0],x[0].replace('`','').split(': ',1)[1]):int(x[1]) for x in ct} == category
    st = table_rows(record,'| Source | Pinned commit |')
    assert len(st) == 10
    for table,s in zip(st,lock['sources'],strict=True):
        assert s['repository'] in table[0] and '/tree/'+s['commit'] in table[0]
        assert table[1] == '`'+s['commit'][:12]+'`'
        assert int(table[2]) == len(s['documents']) and int(table[3]) == expected_source[s['id']]['samples']
    assert '25 repositories across four batches' in record
    assert '443/451 (98.2%)' in record and '268/451 diagnoses (59.4%)' in record
    assert 'Japanese and Chinese each have zero samples' in record
    assert 'final-commit CI and independent stage 7 acceptance\nwere pending' in record
    assert 'CI on the final delivered commit is linked' not in record
    assert 'configuration or navigation blob in the pinned tree' in procedure
    for text in [record,procedure]:
        assert 'Before future integration into `main`' in text or 'Before merging, rebase on `main`' in text
        assert 'byte-identical raw results' in text
        assert 'without\nrelabeling' in text or 'without\nrelabeling it' in text
    assert not (ROOT/'docs/evaluation/lnk002-holdout-plan.md').exists()
    assert b'Delete it in the\nfinal commit of this branch' in git('show',freeze['freeze_commit']+':docs/evaluation/lnk002-holdout-plan.md')
    assert 'replaces the work order' in record
    assert 'PR 56 and' in record and 'overlap on that fix' in record and 'neither has been merged' in record
    assert 'preview status and rule documentation stay at the frozen implementation' in record
    passed('record source/result/cause tables exact; four-batch correction; plan deletion/procedure and future-main replay/PR56 overlap wording consistent')

    implementation = freeze['implementation']
    assert len(implementation) == 60 and implementation == run['implementation'] == report['implementation'] == summary['protocol']['implementation']
    assert summary['protocol']['freeze_commit'] == freeze['freeze_commit'] == '136585a72b7317df0a9893348350d04f70b8de91'
    for path,digest in implementation.items():
        assert sha((ROOT/path).read_bytes().replace(b'\r\n',b'\n')) == digest
        assert sha(git('show',freeze['freeze_commit']+':'+path).replace(b'\r\n',b'\n')) == digest
    assert not git('diff','--name-only',freeze['freeze_commit'],'--','src','docs/rules','examples/evaluate_m2.rs','Cargo.toml','Cargo.lock','corpus/corpus.py','corpus/inventory.py','scripts/evaluation/evaluate_m2.py','scripts/evaluation/fresh_links.py').strip()
    for ancestor,descendant in [(freeze['main_commit'],freeze['freeze_commit']),(freeze['heading_name_fix_commit'],freeze['integration_commit']),(freeze['integration_commit'],freeze['freeze_commit'])]:
        subprocess.run(['git','merge-base','--is-ancestor',ancestor,descendant],cwd=ROOT,check=True)
    main = git('rev-parse','refs/remotes/origin/main').decode().strip()
    remote_main = git('ls-remote','origin','refs/heads/main').decode().split()[0]
    assert main == remote_main == freeze['main_commit'] == '7f8371f25ae438561c28ce99931b666b7723d629'
    prs = []
    for number in [55,56]:
        pr = json.loads(subprocess.check_output(['gh','pr','view',str(number),'--repo','scarletkc/seiso','--json','number,state,isDraft,mergedAt,headRefName,headRefOid,baseRefName,url'],cwd=ROOT))
        assert pr['state']=='OPEN' and pr['isDraft'] is True and pr['mergedAt'] is None and pr['baseRefName']=='main'
        prs.append(pr)
    facts['pull_requests'] = prs
    facts['remote_main'] = remote_main
    facts['implementation_fingerprints_verified'] = len(implementation)
    facts['protected_paths_unchanged'] = True
    facts['future_main_replay_performed'] = False
    passed('all60 frozen implementation fingerprints, protected paths, actual main and both unmerged draft PRs verified')

    V = ROOT/'corpus/results/lnk002/validation'
    for name,n in [('seiso-fresh-links-tests.log.gz',32),('seiso-python-scripts-tests.log.gz',176),('seiso-python-corpus-tests.log.gz',10),('final-oracle-tests.log.gz',11)]:
        content = gzip.decompress((V/name).read_bytes()).decode()
        assert re.search(r'Ran '+str(n)+r' tests\b',content) and '\nOK' in content
        track(V/name)
    rust = gzip.decompress((V/'seiso-tooling-rust-tests-isolated.log.gz').read_bytes()).decode()
    groups = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',rust)
    assert tuple(sum(int(x[i]) for x in groups) for i in range(3)) == (284,0,2)
    track(V/'seiso-tooling-rust-tests-isolated.log.gz')
    for name in ['seiso-tooling-build.log.gz','seiso-tooling-clippy.log.gz','seiso-tooling-docs.log.gz']:
        content=gzip.decompress((V/name).read_bytes()).decode()
        assert 'Finished' in content and 'error:' not in content
        track(V/name)
    compatibility = read(V/'tooling-regression.json')
    for name,digest in compatibility['artifacts'].items():assert sha((V/name).read_bytes())==digest
    before = gzip.decompress((V/'m2-before-raw.json.gz').read_bytes())
    after = gzip.decompress((V/'m2-after-raw.json.gz').read_bytes())
    assert before == after and len(before)==6668402 and sha(before)=='167c842a507c2cf54abaa2d80a76245f20449342aa2641e301b6af40c9b2c249'
    assert (V/'oracle-before.json.gz').read_bytes()==(V/'oracle-after.json.gz').read_bytes()
    regression = read(V/'heading-name-regression.json')
    for name,digest in regression['artifacts'].items():assert sha((V/name).read_bytes())==digest
    heading_raw = gzip.decompress((V/'heading-name-after-raw.json.gz').read_bytes())
    assert sha(heading_raw)==regression['after_raw_sha256']=='55e8c645172c753527f99b4d0fab1317f8b21770f267c67d4336ee68418fbad8'
    assert regression['removed_diagnostics']==32 and regression['added_diagnostics']==0 and regression['all_other_file_results_unchanged'] is True
    raw = gzip.decompress((B/'run/raw-results.json.gz').read_bytes())
    assert len(raw)==35194918 and sha(raw)==run['raw_result_sha256']=='143c270d70e714faf1285729f885b66fe937d5befee2f1a6477c96a0ead4e28e'
    replay = read(B/'run/raw-replay.json')
    facts['retained_validation_counts']={'script_tests_including32focused':176,'corpus_tests':10,'oracle_tests':11,'rust_passed':284,'rust_ignored':2}
    facts['literal_retained_raw_results_sha256'] = sha(raw)
    facts['fresh_evaluation_rerun_by_stage7_reviewer'] = False
    workflow=(ROOT/'.github/workflows/ci.yml').read_text()
    assert 'os: [ubuntu-latest, windows-latest, macos-latest]' in workflow
    passed('retained test counts and historical compatibility/raw hashes verified; CI wording explicitly pending rather than fabricated')

    for p in [F/'manifest.json',F/'summary.json',F/'findings.json',F/'decision.json',F/'freeze.json',F/'sources.json',F/'source-diversity-checkpoint.json',B/'corpus.lock.json',B/'kinds.json',B/'sites.json',B/'inventory/inventory.lock.json',B/'author-annotation-seal.json',B/'independent-annotation-seal.json',B/'stage-6-independent-review.json',B/'stage-6-checkpoint.json',B/'run/diagnostics.json.gz',B/'run/run.json',B/'run/raw-replay.json',B/'run/raw-results.json.gz',A/'verify-stage6-bundle.py',A/'labels-links.json',A/'author-labels.json.gz',A/'independent-labels.json.gz',A/'independent-evidence.tar.gz',A/'archive-receipt.json',A/'reconciliation.json',A/'independent-reconciliation-proposal.json',record_path,procedure_path,V/'tooling-regression.json',V/'heading-name-regression.json',ROOT/'.github/workflows/ci.yml']:
        track(p)
    facts['summary_sha256'] = sha((F/'summary.json').read_bytes())
    facts['resolved_labels_sha256'] = sha((A/'labels-links.json').read_bytes())
    facts['blind_protocol_personally_preserved'] = True
    facts['individual_context_reviews_repeated_in_stage7'] = False
except Exception as exc:
    errors.append({'type':type(exc).__name__,'reason':str(exc),'traceback':traceback.format_exc()})

receipt = {
    'schema_version':1,'stage':7,'reviewer':'/root/blind_lnk002_reviewer',
    'reviewed_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'reviewed_head':git('rev-parse','HEAD').decode().strip(),
    'pass':not errors,'checks':checks,'facts':facts,'hashes':hashes,'errors':errors,
    'review_scope':'Prepared stage7 artifacts and record/procedure; final exact-head CI follows the receipt and final artifact commits.',
    'delivery_checks':{'final_commit_exact_head_ci':'pending; parent must verify after final commits','stage7_artifact_review':'passed' if not errors else 'blocked'},
    'limitations':[
        'Source-diversity owner acceptance remains null; numeric gate independently fails, so preview retention is valid without that approval.',
        'The49 renderer uncertainties remain unchanged:48 VSCode production-renderer cases and one exact Material tab-build case.',
        'Existing complete stage6 proof archive and original review hash-chain were checked for immutability; the451 individually reviewed contexts were not repeated.',
        'Retained test logs and prior raw/reverse receipts were audited. This review does not claim a new browser observation, upstream build, fresh evaluation or final-head CI run.'
    ],
    'verification_script_path':'/tmp/seiso-stage7-independent-verifier.py',
    'verification_script_sha256':sha(Path(__file__).read_bytes())
}
(F/'stage-7-independent-review.json').write_bytes(enc(receipt))
print(json.dumps({'pass':receipt['pass'],'checks':len(checks),'errors':errors,'receipt_sha256':sha((F/'stage-7-independent-review.json').read_bytes())},sort_keys=True))
