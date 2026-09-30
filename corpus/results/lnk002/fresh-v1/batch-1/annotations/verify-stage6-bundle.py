import pathlib,json,gzip,hashlib,tarfile,subprocess,datetime,traceback,sys
sys.path.insert(0,'/tmp/seiso-independent-lnk002');import review
ROOT=pathlib.Path('/workspace/seiso');B=review.B;A=B/'annotations';OUT=review.OUT;checks=[];hashes={};facts={};errors=[]
def sha(raw):return hashlib.sha256(raw).hexdigest()
def read(name):return json.loads((A/name).read_bytes())
def git(*args):return subprocess.check_output(['git',*args],cwd=ROOT)
def passed(name):checks.append({'check':name,'passed':True})
try:
 report=review.REPORT;rows=review.ROWS;rowmap={r['id']:r for r in rows};run=json.loads((B/'run/run.json').read_bytes());checkpoint=json.loads((B/'stage-6-checkpoint.json').read_bytes());assert len(rows)==451==len(rowmap);assert sha(review.PACK)==checkpoint['report_sha256']==run['report_sha256']=='5d674b83996528dfd4702bbd7c15732cc82be29a889e4edf7ab20eb07cd04cac'
 for key,path in [('corpus_sha256',B/'corpus.lock.json'),('inventory_sha256',B/'inventory/inventory.lock.json'),('kind_profile_sha256',B/'kinds.json'),('site_profile_sha256',B/'sites.json')]:assert sha(path.read_bytes())==run[key]==report[key]
 passed('frozen report, corpus, inventory and profiles exact hashes')
 archive_receipt=read('archive-receipt.json');bound=read('labels-links.json');recon=read('reconciliation.json');proposal=read('independent-reconciliation-proposal.json');ind_receipt=read('independent-stage-6-independent-reconciliation-receipt.json');originals={};raworiginals={}
 for name,record in archive_receipt['original_annotations'].items():
  path=A/(name+'.gz');packed=path.read_bytes();raw=gzip.decompress(packed);assert len(packed)==record['archive_bytes'];assert sha(packed)==record['archive_sha256'];assert len(raw)==record['bytes'];assert sha(raw)==record['sha256'];raworiginals[name]=raw;originals[name]=json.loads(raw)
 assert raworiginals['author-labels.json']==pathlib.Path('/tmp/seiso-author-lnk002/author-labels.json').read_bytes();assert raworiginals['independent-labels.json']==(OUT/'independent-labels.json').read_bytes();assert sha(raworiginals['author-labels.json'])=='4aa71a2ca33d84281d5212797f5a864a17668128254a7ae7930d65b85aa1879c';assert sha(raworiginals['independent-labels.json'])=='19f7eddd358f311910b0504485d10219279a1219057d79c4431dd3b63541cc0d'
 assert bound['original_annotations']==archive_receipt['original_annotations'];assert bound['human_reviewers']==archive_receipt['human_reviewers']==recon['human_reviewers']==0
 passed('all compressed original/archive byte hashes and both annotation seals')
 author={x['id']:x for x in originals['author-labels.json']['labels']};independent={x['id']:x for x in originals['independent-labels.json']['labels']};labels={x['id']:x for x in bound['labels']};assert len(author)==len(independent)==len(labels)==451;assert set(author)==set(independent)==set(labels)==set(rowmap);assert set(bound['agent_context_reviewed_ids'])==set(rowmap) and len(bound['agent_context_reviewed_ids'])==451
 binding_fields=['id','code','source','path','span','input_sha256','related_inputs','split'];nested_fields=['label','reason','renderer','evidence','reviewer','agent_context_reviewed'];differences=[]
 for id,row in rowmap.items():
  final=labels[id];aa=author[id];ii=independent[id]
  for record in [final,aa,ii]:
   for key in binding_fields:assert record[key]==row[key],(id,key)
   assert record['diagnostic_sha256']==sha(review.enc(row['diagnostic']));assert record['agent_context_reviewed'] is True;assert record['label'] in ['tp','fp','uncertain']
   for key in ['reason','renderer','evidence']:assert isinstance(record[key],str) and record[key].strip()
  assert final['author_review']=={k:aa[k] for k in nested_fields};assert final['independent_review']=={k:ii[k] for k in nested_fields};assert aa['reviewer']=='/root';assert ii['reviewer']=='/root/blind_lnk002_reviewer';assert final['review_index']==ii['review_index']==aa['review_index'];assert review.ROWS[final['review_index']]['id']==id
  if aa['label']!=ii['label']:differences.append(id)
  else:assert final['label']==aa['label']==ii['label']
 assert len(differences)==8;assert len(rowmap)-len(differences)==443
 passed('451 exact report bindings and unchanged complete nested agent reviews')
 assert recon['original_author_sha256']==sha(raworiginals['author-labels.json']);assert recon['original_independent_sha256']==sha(raworiginals['independent-labels.json']);assert recon['both_originals_unchanged'] is True;assert recon['original_agreements']==443 and recon['original_disagreements']==8
 resolutions={x['id']:x for x in recon['resolutions']};proposals={x['id']:x for x in proposal['proposals']};assert set(resolutions)==set(proposals)==set(differences);assert (A/'independent-reconciliation-proposal.json').read_bytes()==(OUT/'reconciliation-proposal.json').read_bytes();assert (A/'independent-stage-6-independent-reconciliation-receipt.json').read_bytes()==(OUT/'stage-6-independent-reconciliation-receipt.json').read_bytes()
 for id,p in proposals.items():
  resolution=resolutions[id];final=labels[id]
  for k,v in p.items():assert resolution[k]==v,(id,k)
  assert resolution['resolved_label']==p['proposed_resolved_label']==final['label'];assert resolution['implementer_proposal_accepted'] is True;assert resolution['author_original_unchanged'] is True and resolution['independent_original_unchanged'] is True
  for key in ['reason','renderer']:assert final[key]==p[key],(id,key)
  assert final['evidence']==p['evidence']+' Full primary proof retained in annotations/independent-evidence.tar.gz; reconciliation-proposal.json binds this case.'
  assert p['limitations'] in final['disagreement_reason'];assert p['reason'] in final['disagreement_reason']
 assert {p['review_index']:p['proposed_resolved_label'] for p in proposals.values()}=={0:'fp',45:'uncertain',188:'fp',301:'fp',302:'fp',317:'fp',378:'fp',379:'fp'}
 passed('all eight resolutions exact accepted proposal and conservative tab uncertainty')
 decisions_packed=(A/'decisions-links.json.gz').read_bytes();decisions_raw=gzip.decompress(decisions_packed);decisions=json.loads(decisions_raw);assert sha(decisions_raw)==archive_receipt['decision_uncompressed_sha256']==bound['decision_receipt_sha256']=='7003b3d8e31b1914734963d1a04e0f1073349557008805b4da8eeec25678ebd7';assert decisions['labels']==bound['labels']
 for k,v in decisions.items():assert bound[k]==v,(k,'decision metadata')
 assert bound['report_sha256']==checkpoint['report_sha256'];assert sha((A/'labels-links.json').read_bytes())==checkpoint['resolved_labels_sha256']=='29fa5f78b40212a994eb32e41105cded73738c422d0384ac8b4c9f1033e44c23';assert sha((A/'independent-stage-6-independent-reconciliation-receipt.json').read_bytes())==checkpoint['stage_6_independent_reconciliation_receipt_sha256']=='24a984a73058a890c6b8037b109d1751bd62ffcc9e35e9d2a198f5ab86abc1ee'
 passed('bound-label payload and original decision-byte binding')
 candidate=originals['candidate.json'];oracle=originals['oracle.json'];assert candidate==report['files'];assert len(candidate)==2230;assert oracle['candidate_sha256']==sha(raworiginals['candidate.json']);assert oracle['corpus_sha256']==run['corpus_sha256'];assert oracle['inventory_sha256']==run['inventory_sha256'];assert oracle['human_reviewers']==0;assert len(oracle['labels'])==451 and {x['id'] for x in oracle['labels']}==set(rowmap)
 for x in oracle['labels']:
  row=rowmap[x['id']]
  for k in ['id','code','source','path','span','input_sha256','split']:assert x[k]==row[k]
  assert x['agent_context_reviewed'] is False
 passed('exact frozen candidate files and explicitly preliminary oracle binding')
 # Read inert archive members; never extract or execute packaged code.
 archive=A/'independent-evidence.tar.gz';packed=archive.read_bytes();assert len(packed)==archive_receipt['independent_evidence_archive_bytes'];assert sha(packed)==archive_receipt['independent_evidence_archive_sha256'];tarhash={};tarbytes={};special={'independent-seal.json','evidence-manifest.json','reconciliation-proposal.json','reconciliation-evidence/inputs.json','stage-6-independent-reconciliation-receipt.json','validation.json','independent-labels.json'}
 with tarfile.open(archive,'r:gz') as t:
  members=t.getmembers();assert [pathlib.PurePosixPath(m.name) for m in members]==sorted(pathlib.PurePosixPath(m.name) for m in members);assert len({m.name for m in members})==len(members)
  for m in members:
   assert m.isfile() and not m.issym() and not m.islnk();assert m.uid==m.gid==m.mtime==0;assert m.mode==0o644;assert m.uname==m.gname=='';p=pathlib.PurePosixPath(m.name);assert not p.is_absolute() and '..' not in p.parts and '__pycache__' not in p.parts
   raw=t.extractfile(m).read();assert len(raw)==m.size;tarhash[m.name]=sha(raw);assert (OUT/m.name).is_file();assert sha((OUT/m.name).read_bytes())==tarhash[m.name],m.name
   if m.name in special:tarbytes[m.name]=raw
 expected={p.relative_to(OUT).as_posix() for p in OUT.rglob('*') if p.is_file() and '__pycache__' not in p.parts};assert set(tarhash)==expected,(len(tarhash),len(expected),sorted(expected-set(tarhash)))
 manifest=read('independent-evidence-manifest.json');assert tarbytes['evidence-manifest.json']==(A/'independent-evidence-manifest.json').read_bytes()
 for entry in manifest['files']:assert tarhash[entry['path']]==entry['sha256']
 seal=json.loads(tarbytes['independent-seal.json']);assert seal['count']==451 and seal['human_reviewers']==0;assert seal['labels_sha256']==tarhash['independent-labels.json']==sha(raworiginals['independent-labels.json']);assert seal['evidence_manifest_sha256']==tarhash['evidence-manifest.json'];assert seal['validation_sha256']==tarhash['validation.json'];assert tarbytes['validation.json']==(A/'independent-validation.json').read_bytes();assert tarbytes['reconciliation-proposal.json']==(A/'independent-reconciliation-proposal.json').read_bytes();assert tarbytes['stage-6-independent-reconciliation-receipt.json']==(A/'independent-stage-6-independent-reconciliation-receipt.json').read_bytes();assert tarhash['reconciliation-evidence/inputs.json']==proposal['evidence_inputs_sha256']==ind_receipt['primary_evidence_sha256'];assert ind_receipt['proposal_sha256']==tarhash['reconciliation-proposal.json'];assert ind_receipt['blind_protocol_preserved'] is True and ind_receipt['originals_modified'] is False
 proof=json.loads(tarbytes['reconciliation-evidence/inputs.json'])
 for item in proof['files']:assert tarhash[item['path']]==item['sha256']
 for item in proof['exact_bundle_excerpts']:assert tarhash[item['retained_path']]==item['excerpt_sha256']
 for item in proof['bindings']:
  assert tarhash[item['source_retained_path']]==item['source_sha256'];assert tarhash[item['target_retained_path']]==item['target_sha256']
 assert {x['id'] for x in proof['bindings']}==set(differences);assert all(f'contexts/{i:03}.json' in tarhash for i in range(451));facts['archive_members']=len(tarhash);facts['original_manifest_files']=len(manifest['files']);facts['reconciliation_proof_files']=len(proof['files']);facts['archive_metadata']='path-component-sorted regular files; uid/gid/mtime0; empty owner names; mode0644; no links/unsafe paths/bytecode cache'
 passed('complete deterministic archive, original manifests and all post-seal proof hashes')
 frozen=checkpoint['freeze_commit'];assert frozen==run['freeze_commit']==report['freeze_commit']=='136585a72b7317df0a9893348350d04f70b8de91';paths={'Cargo.lock','Cargo.toml','scripts/evaluation/evaluate_m2.py','scripts/evaluation/fresh_links.py','corpus/corpus.py','corpus/inventory.py','examples/evaluate_m2.rs'};paths|={p.relative_to(ROOT).as_posix() for p in (ROOT/'src').rglob('*.rs')};paths|={p.relative_to(ROOT).as_posix() for p in (ROOT/'docs/rules').glob('*.md')};assert paths==set(run['implementation']) and len(paths)==60
 for path,expected_sha in run['implementation'].items():assert sha((ROOT/path).read_bytes().replace(b'\r\n',b'\n'))==expected_sha;assert sha(git('show',f'{frozen}:{path}').replace(b'\r\n',b'\n'))==expected_sha
 protected_diff=git('diff','--name-only',frozen,'--','src','docs/rules','Cargo.toml','Cargo.lock','scripts/evaluation/evaluate_m2.py','scripts/evaluation/fresh_links.py','corpus/corpus.py','corpus/inventory.py','examples/evaluate_m2.rs').decode().strip();assert not protected_diff
 main=git('rev-parse','refs/remotes/origin/main').decode().strip();remote_main=git('ls-remote','origin','refs/heads/main').decode().split()[0];assert main==remote_main=='7f8371f25ae438561c28ce99931b666b7723d629';facts['implementation_files']=60;facts['remote_main']=remote_main
 passed('all60 implementation fingerprints equal freeze; protected paths and main unchanged')
 assert checkpoint['labels']==451 and checkpoint['original_agreements']==443 and checkpoint['original_disagreements']==8 and checkpoint['human_reviewers']==0;assert checkpoint['all_labels_context_reviewed_by_both_agents'] is True;assert checkpoint['unique_exact_identity_and_diagnostic_payload_binding'] is True;assert checkpoint['implementation_unchanged'] is True;assert checkpoint['both_originals_unchanged'] is True;assert checkpoint['oracle_labels_are_preliminary_only'] is True
 passed('stage6 checkpoint claims independently supported')
 facts.update(labels=451,original_agreements=443,original_disagreements=8,individual_context_reviews_repeated=False,human_reviewers=0,blind_protocol_preserved=True,originals_byte_identical_to_seals=True)
 for p in sorted(A.iterdir()):
  if p.is_file():hashes[p.relative_to(B).as_posix()]=sha(p.read_bytes())
 hashes['stage-6-checkpoint.json']=sha((B/'stage-6-checkpoint.json').read_bytes());hashes['run/diagnostics.json.gz']=sha(review.PACK);hashes['original_uncompressed_author_labels']=sha(raworiginals['author-labels.json']);hashes['original_uncompressed_independent_labels']=sha(raworiginals['independent-labels.json']);hashes['original_uncompressed_decisions']=sha(decisions_raw);hashes['archive_reconciliation_evidence_inputs']=tarhash['reconciliation-evidence/inputs.json']
except Exception as exc:
 errors.append({'type':type(exc).__name__,'reason':str(exc),'traceback':traceback.format_exc()})
receipt={'schema_version':1,'stage':6,'reviewer':'/root/blind_lnk002_reviewer','reviewed_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'reviewed_head':git('rev-parse','HEAD').decode().strip(),'pass':not errors,'checks':checks,'facts':facts,'hashes':hashes,'errors':errors,'limitations':['VS Code production-renderer uncertainty and exact Material tab-build uncertainty remain explicit in the accepted labels.','This validates the assembled evidence and bindings; it does not repeat the451 original individual reviews or run upstream build/config code.']};path=B/'stage-6-independent-review.json';path.write_bytes(review.enc(receipt));print(json.dumps({'pass':not errors,'receipt':str(path),'receipt_sha256':sha(path.read_bytes()),'checks_passed':len(checks),'errors':errors,'facts':facts},indent=2));sys.exit(1 if errors else 0)
