import datetime, hashlib, json, os, pathlib, re, resource, subprocess, sys

root = pathlib.Path('/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-artifacts')
wrapper = pathlib.Path('/tmp/nyxid-artifacts-2037-validation.py')
identity_path = pathlib.Path('/tmp/nyxid-artifacts-2037-compile-8544ce193-20261027T104256Z.binary.json')
identity = json.loads(identity_path.read_text())
binary = pathlib.Path(identity['binary'])
handler_prefix = 'handlers::mcp_transport::chat_authority_tests::async_service_operation_tests::generated_artifact_delivery_tests::'
service_prefix = 'services::generated_artifacts::tests::'
handler_names = [
    'generated_artifact_worker_rejects_expired_changed_and_unavailable_sources',
    'generated_artifact_worker_reserves_shared_budget_before_result_body_and_releases_on_cancel',
]
service_names = [
    'maximum_encrypted_pending_document_fits_bson_and_rejects_oversize',
    'generated_artifacts_materialize_history_owner_http_ranges_recovery_and_retention',
    'generated_artifacts_exact_event_binding_cancellation_capacity_and_recovery',
    'generated_artifacts_expiry_and_stale_lease_cannot_publish_or_relabel',
    'generated_artifact_worker_reservation_spans_persist_and_releases_on_error',
    'owner_http_preserves_legacy_document_upload_and_image_contracts',
    'delivery_expiry_uses_effective_attachment_retention_and_recovery_checks_live_policy',
    'generated_artifact_admission_abort_keeps_pending_bytes_and_rolls_back_retention_and_association',
    'generated_artifact_new_operation_has_new_identity_and_failed_result_preserves_old_bytes',
    'conversation_delete_removes_encrypted_pending_artifacts_and_saved_associations',
]
selections = [handler_prefix + name for name in handler_names] + [service_prefix + name for name in service_names]
stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
batch_path = pathlib.Path('/tmp/nyxid-artifacts-2037-focused-8544ce193-batch-' + stamp + '.json')
batch = {'started_utc':stamp, 'source_head':identity['source_head'], 'source_digest_sha256':identity['source_digest_sha256'], 'executable_identity':identity, 'selections':selections, 'results':[], 'status':'running'}

batch['approved_order_numbers'] = [4,5,7,8,9,10,11,12,13,14,15,16]
batch['skipped'] = [{'approved_order_number':6,'reason':'explicit user instruction: already passed on 2af; skip duplicate','metadata':'/tmp/nyxid-artifacts-2037-regression-01-20261027T100859Z.json'}]
assert os.environ.get('RUST_MIN_STACK') is None, 'Default stack required'
assert resource.getrlimit(resource.RLIMIT_NOFILE)[0] == 65536
assert identity['source_head'] == '8544ce1934251e3bc1abd223014af8c76375397d'
assert len(selections) == 12

assert identity['binary_sha256'] == '58a90e02f55dbba3dc925ca23f85e6ec2050a105b1a932b737c58c34ed46600b'
batch['carried_forward'] = [
    {'approved_order_number': 1, 'metadata': '/tmp/nyxid-artifacts-2037-focused-90f4c75a2-01-20261027T103720Z.json', 'reason': 'explicitly reviewed and carried forward; correction changes only malformed test phases'},
    {'approved_order_number': 2, 'metadata': '/tmp/nyxid-artifacts-2037-focused-90f4c75a2-02-20261027T103725Z.json', 'reason': 'explicitly reviewed and carried forward; correction changes only malformed test phases'},
    {'approved_order_number': 3, 'metadata': '/tmp/nyxid-artifacts-2037-regression-malformed-8544ce193-20261027T105310Z.json', 'reason': 'current-source pass independently reviewed'}
]

def save():
    batch_path.write_text(json.dumps(batch, indent=2) + '\n')

def executable_hash():
    digest = hashlib.sha256()
    with binary.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()

save()
print('BATCH_METADATA ' + str(batch_path), flush=True)
for number, selection in zip(batch['approved_order_numbers'], selections, strict=True):
    head = subprocess.check_output(['git','rev-parse','HEAD'], cwd=root, text=True).strip()
    if head != identity['source_head'] or subprocess.check_output(['git','status','--porcelain'], cwd=root, text=True).strip() or executable_hash() != identity['binary_sha256']:
        batch['status'] = 'identity_mismatch'
        save()
        raise SystemExit('Refusing test: source or compiled executable identity changed')
    command = [str(binary), selection, '--exact', '--test-threads=1', '--nocapture']
    label = 'focused-8544ce193-' + format(number, '02d')
    print('SELECTION ' + str(number) + '/' + str(len(selections)) + ' ' + selection, flush=True)
    proc = subprocess.Popen([sys.executable, str(wrapper), label, *command], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    meta_path = None
    for line in proc.stdout:
        print(line, end='', flush=True)
        if line.startswith('METADATA '):
            meta_path = pathlib.Path(line.removeprefix('METADATA ').strip())
    wrapper_exit = proc.wait()
    if meta_path is None:
        batch['status'] = 'wrapper_failed'
        batch['results'].append({'selection':selection,'wrapper_exit_code':wrapper_exit})
        save()
        raise SystemExit(wrapper_exit or 65)
    meta = json.loads(meta_path.read_text())
    meta['executable_identity'] = identity
    meta['executable_identity_record'] = str(identity_path)
    meta['selection'] = selection
    meta['approved_order_number'] = number
    meta['binary_sha256_verified'] = identity['binary_sha256']
    meta['default_stack'] = True
    meta['env']['RUST_MIN_STACK'] = None
    meta['wrapper_exit_code'] = wrapper_exit
    log_text = pathlib.Path(meta['log']).read_text(errors='replace')
    one_passed = re.search(r'^running 1 test$', log_text, re.M) is not None and re.search(r'^test result: ok\. 1 passed; 0 failed;', log_text, re.M) is not None
    meta['exactly_one_test_passed'] = one_passed
    meta['source_matches_compiled_identity'] = meta['source_head'] == identity['source_head'] and meta['source_digest_sha256'] == identity['source_digest_sha256']
    meta_path.write_text(json.dumps(meta, indent=2) + '\n')
    result = {'selection':selection,'metadata':str(meta_path),'log':meta['log'],'argv_log':meta['argv_log'],'exit_code':meta.get('exit_code'),'wrapper_exit_code':wrapper_exit,'exactly_one_test_passed':one_passed,'source_matches_compiled_identity':meta['source_matches_compiled_identity'],'approved_order_number':number,'disk_guard_stopped':meta.get('disk_guard_stopped'),'lowest_free_bytes':meta.get('lowest_free_bytes')}
    batch['results'].append(result)
    save()
    if wrapper_exit != 0 or meta.get('exit_code') != 0:
        batch['status'] = 'failed'
        batch['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
        print('STOP_FIRST_NONZERO ' + json.dumps(result), flush=True)
        raise SystemExit(wrapper_exit or 65)
    if not one_passed or not meta['source_matches_compiled_identity']:
        batch['status'] = 'verification_failed'
        save()
        raise SystemExit('Refusing to continue: expected exactly one passing test on the compiled source')
    print('PASS ' + selection, flush=True)
batch['status'] = 'passed'
batch['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
save()
print('BATCH_PASS ' + str(len(selections)) + ' tests; ' + str(batch_path), flush=True)
