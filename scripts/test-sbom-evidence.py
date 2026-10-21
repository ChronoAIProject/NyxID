#!/usr/bin/env python3
"""Offline schema/closure negatives plus real pinned Syft+regctl filesystem replay."""
import base64
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import image_sbom
import image_publication_sboms as images
import release_sboms
from publication_contract import file_digest, source_identity
from sbom_evidence import bind_outputs, document, include, load_json, validate, verify_outputs, write
from test_publication_contract import cli_plan, images_plan

COMMIT = 'a' * 40
SOURCE = source_identity(COMMIT, 1792367116)


def simple(name='retained', digest='sha256:' + '1' * 64):
    result = document(name, digest, COMMIT)
    result['components'] = [dict(type='file', name='actual.js', **{'bom-ref': 'retained-file'},
                                 hashes=[dict(alg='SHA-256', content='2' * 64)])]
    result['dependencies'][0]['dependsOn'] = ['retained-file']
    return result


def platform_bundle(directory, component, architecture):
    directory.mkdir(parents=True)
    config = json.dumps(dict(os='linux', architecture=architecture,
        config={'Labels': {'org.opencontainers.image.revision': COMMIT, 'io.nyxid.test.component': component}})).encode()
    config_id = 'sha256:' + hashlib.sha256(config).hexdigest()
    manifest = json.dumps(dict(schemaVersion=2, mediaType='application/vnd.oci.image.manifest.v1+json',
        config=dict(mediaType='application/vnd.oci.image.config.v1+json', digest=config_id, size=len(config)),
        layers=[dict(mediaType='application/vnd.oci.image.layer.v1.tar', digest='sha256:'+'3'*64, size=1024)])).encode()
    digest = 'sha256:' + hashlib.sha256(manifest).hexdigest()
    scan = dict(component=component, platform='linux/'+architecture, source_commit=COMMIT,
        image_id=config_id, subject=dict(digest=digest, mediaType='application/vnd.oci.image.manifest.v1+json'))
    artifact = dict(name='fixture', version='1.0', type='apk', purl='pkg:apk/alpine/fixture@1.0')
    catalog = dict(source=dict(type='image', metadata=dict(imageID=config_id, manifestDigest=digest,
        architecture=architecture, os='linux', config=base64.b64encode(config).decode(), manifest=base64.b64encode(manifest).decode())),
        descriptor=dict(name='syft', version='1.54.1', configuration=dict(search=dict(scope='squashed'))), artifacts=[artifact])
    filesystem = simple('filesystem')
    filesystem['components'] = [dict(type='library', name='fixture', version='1.0', purl=artifact['purl'], **{'bom-ref':'pkg'})]
    filesystem['dependencies'][0]['dependsOn']=['pkg']
    names=['filesystem.syft.json','filesystem.cdx.json','image.cdx.json'];children={}
    package=image_sbom.IMAGES[component][2];binary_path=image_sbom.IMAGES[component][3]
    if package:
        binary=directory/Path(binary_path).name;binary.write_bytes(b'final synthetic '+component.encode())
        target={'amd64':'x86_64','arm64':'aarch64'}[architecture]+'-unknown-linux-'+image_sbom.IMAGES[component][4]
        info=dict(schema='nyxid.rust-binary-info.v1',subject=dict(name=binary.name,size=binary.stat().st_size,sha256=file_digest(binary)[7:]),
            build_identity=dict(schema='nyxid.build-identity.v1',package=package,package_version='1.0',target=target,commit=COMMIT,source_date_epoch=SOURCE['epoch'],source_state='exported'),
            audit=dict(format=1,packages=[dict(name=package,version='1.0',source='local',root=True,dependencies=[1]),dict(name='fixture-rust',version='1.0',source='crates.io')]))
        file_catalog=dict(descriptor=dict(name='syft',version='1.54.1'),source=dict(type='file',metadata=dict(digests=[dict(algorithm='sha256',value=info['subject']['sha256'])])),artifacts=[])
        for i,p in enumerate(info['audit']['packages']):
            raw=dict(name=p['name'],version=p['version'],metadata=p,foundBy='cargo-auditable-binary-cataloger',metadataType='rust-cargo-audit-entry',locations=[dict(path=binary_path)])
            file_catalog['artifacts'].append(raw);catalog['artifacts'].append(raw)
            filesystem['components'].append(dict(type='library',name=p['name'],version=p['version'],**{'bom-ref':'rust-'+str(i)}))
            filesystem['dependencies'][0]['dependsOn'].append('rust-'+str(i))
        child=image_sbom.archives.binary_sbom.sbom(binary,info,package,COMMIT)
        write(directory/'binary-info.json',info);write(directory/'binary.syft.json',file_catalog)
        if package=='nyxid-cli':
            seal=(image_sbom.archives.binary_sbom.ROOT/'cli/src/wizard/assets/shipped-composition.json').read_bytes()
            embedded=dict(schema='nyxid.embedded-wizard.v1',subject=info['subject'],seal_sha256=hashlib.sha256(seal).hexdigest())
            (directory/'wizard-seal.json').write_bytes(seal);write(directory/'embedded-wizard.json',embedded)
            image_sbom.archives.binary_sbom.attach_wizard(child,seal,embedded)
            names+=['wizard-seal.json','embedded-wizard.json']
        write(directory/'binary.cdx.json',child);children['binary']=child
        names+=['binary-info.json','binary.syft.json','binary.cdx.json'];binary.unlink()
    if component in ('backend','frontend'):
        assets=simple('verified-assets');write(directory/'assets.cdx.json',assets);children['assets']=assets;names.append('assets.cdx.json')
    write(directory/'filesystem.syft.json',catalog);write(directory/'filesystem.cdx.json',filesystem)
    write(directory/'image.cdx.json',image_sbom.assembled(scan,filesystem,children))
    receipt=dict(schema='nyxid.image-sbom.v1',component=component,platform=scan['platform'],source=SOURCE,
        subject=scan['subject'],image_id=config_id,archive_sha256='4'*64,children=sorted(children),outputs=bind_outputs(directory,names))
    write(directory/'image-sbom.json',receipt)
    return receipt


class SbomEvidence(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup); self.root=Path(temp.name)

    def test_official_offline_schema_and_closing_dependency_graph(self):
        result=simple(); self.assertEqual(validate(result,result['metadata']['component']['bom-ref'],COMMIT),result)

    def test_wrong_subject_source_schema_and_duplicate_properties_fail(self):
        for kind in ('subject','source','schema','property','hash'):
            d=simple()
            if kind=='subject': d['metadata']['component']['bom-ref']='sha256:'+'0'*64
            elif kind=='source': d['metadata']['component']['properties'][0]['value']='b'*40
            elif kind=='schema': d['components'][0]['type']='imaginary'
            elif kind=='property': d['metadata']['component']['properties']*=2
            else:d['metadata']['component']['hashes'][0]['content']='f'*64
            with self.subTest(kind=kind),self.assertRaises(ValueError): validate(d,'sha256:'+'1'*64,COMMIT)

    def test_missing_duplicate_dangling_and_disconnected_inventory_fail(self):
        for kind in ('missing','duplicate','dangling','disconnected','duplicate-edge','duplicate-node','dangling-provides','duplicate-provides'):
            d=simple()
            if kind=='missing': d['components']=[]
            elif kind=='duplicate': d['components']*=2
            elif kind=='dangling': d['dependencies'][0]['dependsOn']=['absent']
            elif kind=='disconnected':d['dependencies'][0]['dependsOn']=[]
            elif kind=='duplicate-edge':d['dependencies'][0]['dependsOn']*=2
            elif kind=='duplicate-node':d['dependencies']*=2
            elif kind=='dangling-provides':d['dependencies'][0]['provides']=['absent']
            else:d['dependencies'][0]['provides']=['retained-file','retained-file']
            with self.subTest(kind=kind),self.assertRaises(ValueError):validate(d,'sha256:'+'1'*64,COMMIT)

    def test_namespaced_children_preserve_both_platform_graphs(self):
        d=document('index','sha256:'+'9'*64,COMMIT)
        include(d,simple(),'amd64:');include(d,simple(),'arm64:')
        validate(d,'sha256:'+'9'*64,COMMIT)
        self.assertEqual(len(d['components']),4)

    def test_composition_source_uses_fresh_locked_hook_free_frontend_working_directory(self):
        from sbom_evidence import composition_source
        source=self.root/'source';(source/'frontend').mkdir(parents=True);(source/'scripts').mkdir()
        (source/'frontend/package-lock.json').write_text('immutable-lock')
        (source/'frontend/node_modules').mkdir();(source/'frontend/node_modules/untrusted').write_text('old')
        def execute(command,**kwargs):
            if command[1]=='ci':
                self.assertEqual(kwargs['cwd'].name,'frontend')
                self.assertEqual((kwargs['cwd']/'package-lock.json').read_text(),'immutable-lock')
                self.assertFalse((kwargs['cwd']/'node_modules').exists())
                self.assertIn('--ignore-scripts',command);self.assertNotIn('--prefix',command)
                self.assertNotIn('NPM_CONFIG_IGNORE_SCRIPTS',kwargs['env'])
            return subprocess.CompletedProcess(command,0)
        with patch.object(subprocess,'run',side_effect=execute),patch.dict(os.environ,NPM_CONFIG_IGNORE_SCRIPTS='false'),composition_source(source) as exported:
            self.assertEqual((exported/'frontend/package-lock.json').read_text(),'immutable-lock')
        self.assertEqual((source/'frontend/node_modules/untrusted').read_text(),'old')

    def test_output_change_missing_and_symlink_fail(self):
        p=self.root/'a.json';p.write_text('{}');record=bind_outputs(self.root,['a.json'])
        p.write_text('{"changed":true}')
        with self.assertRaises(ValueError):verify_outputs(self.root,record)
        p.unlink()
        with self.assertRaises((ValueError,OSError)):verify_outputs(self.root,record)
        outside=self.root/'b.json';outside.write_text('{}');p.symlink_to(outside)
        with self.assertRaises(ValueError):verify_outputs(self.root,record)

    def test_missing_asset_composition_or_changed_runtime_inventory_fails(self):
        d=self.root/'platform';r=platform_bundle(d,'frontend','amd64');image_sbom.verify_bundle(d)
        r['outputs'].pop('assets.cdx.json');write(d/'image-sbom.json',r)
        with self.assertRaisesRegex(ValueError,'composition'):image_sbom.verify_bundle(d)
        receipt=dict(audit=dict(packages=[dict(name='application',version='1',source='local')]))
        with self.assertRaisesRegex(ValueError,'runtime inventory'):image_sbom.runtime_inventory({'artifacts':[]},receipt,'/app')

    def test_converted_sbom_cannot_drop_or_change_filesystem_packages(self):
        for converted in ({'components':[]},{'components':[dict(name='fixture',version='2',purl='pkg:apk/alpine/fixture@1')]}):
            with self.assertRaises(ValueError):image_sbom.verify_conversion({'artifacts':[dict(name='fixture',version='1',purl='pkg:apk/alpine/fixture@1')]},converted)

    def test_known_syft_file_omissions_are_preserved_without_repairing_package_loss(self):
        packages = [dict(name='webpki-roots', version=version, purl='pkg:cargo/webpki-roots@' + version)
                    for version in ('0.26.11', '1.0.10')]
        files = [dict(id='file-' + str(i), location=dict(path='/lib/' + str(i)), **extra)
                 for i, extra in enumerate(({}, {'metadata': {'type': 'SymbolicLink', 'linkDestination': 'target'}},
                                            {'metadata': {'type': 'Directory'}}, {'metadata': {'type': 'Socket'}}))]
        files[0]['digests'] = [dict(algorithm='sha256', value='2' * 64)]
        raw = dict(artifacts=packages, files=files)
        converted = dict(components=[dict(p, type='library', **{'bom-ref': str(i)}) for i, p in enumerate(packages)])
        with self.assertRaisesRegex(ValueError, 'actual package identities'):
            image_sbom.verify_conversion(raw, converted)
        image_sbom.complete_file_inventory(raw, converted)
        image_sbom.verify_conversion(raw, converted)
        self.assertEqual(converted['components'][2]['hashes'], [dict(alg='SHA-256', content='2' * 64)])
        self.assertEqual([c['bom-ref'] for c in converted['components'][2:]], [f['id'] for f in files])
        self.assertIn('linkDestination', str(converted['components'][3]['properties']))
        for kind in ('drop', 'version', 'purl', 'duplicate'):
            changed = copy.deepcopy(converted)
            if kind == 'drop': changed['components'].pop(0)
            elif kind == 'version': changed['components'][0]['version'] = '1.0.10'
            elif kind == 'purl': changed['components'][0]['purl'] += '?changed=1'
            else: changed['components'].append(copy.deepcopy(changed['components'][0]))
            image_sbom.complete_file_inventory(raw, changed)
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, 'actual package identities'):
                image_sbom.verify_conversion(raw, changed)
        # Losing an ordinary file is not the known Syft omission and remains fatal.
        raw['files'].append(dict(id='regular', location=dict(path='/actual-file'), metadata=dict(type='RegularFile')))
        image_sbom.complete_file_inventory(raw, converted)
        with self.assertRaisesRegex(ValueError, '/actual-file'):
            image_sbom.verify_conversion(raw, converted)

    def test_platform_manifest_config_and_scope_changes_fail(self):
        d=self.root/'platform';r=platform_bundle(d,'frontend','amd64');catalog=load_json(d/'filesystem.syft.json')
        for kind in ('manifest','config','scope','platform','tool'):
            c=copy.deepcopy(catalog)
            if kind in ('manifest','config'):c['source']['metadata'][kind]=base64.b64encode(b'wrong').decode()
            elif kind=='scope':c['descriptor']['configuration']['search']['scope']='all-layers'
            elif kind=='platform':c['source']['metadata']['architecture']='arm64'
            else:c['descriptor']['version']='unreviewed'
            with self.subTest(kind=kind),self.assertRaises(ValueError):image_sbom.verify_catalog(c,r)

    def test_index_binds_both_exact_manifest_digests_and_packages(self):
        paths=[self.root/a for a in ('amd64','arm64')]
        for p in paths:platform_bundle(p,'frontend',p.name)
        receipt=images.index('frontend',SOURCE,paths,self.root/'index')
        self.assertEqual({r['platform'] for r in receipt['subjects']['runnable']},set(images.PLATFORMS))
        self.assertEqual(images.verify_index(self.root/'index'),receipt)
        d=load_json(self.root/'index/index.cdx.json');self.assertEqual(sum(c.get('name')=='fixture' for c in d['components']),2)

    def test_index_rejects_missing_duplicate_source_or_component_platform(self):
        a=self.root/'amd64';b=self.root/'arm64';platform_bundle(a,'frontend','amd64');platform_bundle(b,'frontend','arm64')
        for paths,source,component in (([a],SOURCE,'frontend'),([a,a],SOURCE,'frontend'),([a,b],source_identity('b'*40,SOURCE['epoch']),'frontend'),([a,b],SOURCE,'backend')):
            with self.subTest(paths=paths,source=source,component=component),self.assertRaises(ValueError):images.index(component,source,paths,self.root/'index')

    def test_changed_index_or_composition_cannot_be_accepted_by_rehashing_receipt(self):
        a=self.root/'amd64';b=self.root/'arm64';platform_bundle(a,'frontend','amd64');platform_bundle(b,'frontend','arm64')
        output=self.root/'index';r=images.index('frontend',SOURCE,[a,b],output)
        d=load_json(output/'index.cdx.json');d['components'].pop();write(output/'index.cdx.json',d)
        r['outputs']=bind_outputs(output,r['outputs']);write(output/'index-sbom.json',r)
        with self.assertRaises(ValueError):images.verify_index(output)

    def test_complete_image_publication_rejects_partial_and_extra_components(self):
        paths=[self.root/a for a in ('amd64','arm64')]
        for p in paths:platform_bundle(p,'frontend',p.name)
        evidence=self.root/'evidence';evidence.mkdir();images.index('frontend',SOURCE,paths,evidence/'frontend')
        with self.assertRaisesRegex(ValueError,'incomplete'):images.closure(images_plan(),evidence)
        shutil.copytree(evidence/'frontend',evidence/'duplicate')
        with self.assertRaisesRegex(ValueError,'duplicate'):images.closure(images_plan(),evidence)

    def test_unplanned_auxiliary_platform_cannot_silently_enter_publication_set(self):
        evidence=self.root/'platforms';evidence.mkdir()
        platform_bundle(evidence/'extra','machine-node-only','amd64')
        with self.assertRaisesRegex(ValueError,'extra component'):
            images.generate_set(images_plan(),evidence,self.root/'indices')
        self.assertFalse((self.root/'indices').exists())

    def test_all_ten_platforms_and_five_indices_close_and_only_then_promote(self):
        plan=images_plan();evidence=self.root/'indices';evidence.mkdir()
        for component in sorted({s['component'] for s in plan['subjects']}):
            paths=[self.root/(component+'-'+a) for a in ('amd64','arm64')]
            for a,path in zip(('amd64','arm64'),paths):platform_bundle(path,component,a)
            images.index(component,plan['source'],paths,evidence/component)
        receipt=images.closure(plan,evidence);self.assertEqual(len(receipt['subjects']),15)
        spec=importlib.util.spec_from_file_location('promote',Path(__file__).with_name('promote-sbom-images.py'))
        promote=importlib.util.module_from_spec(spec);spec.loader.exec_module(promote)
        digest=receipt['subjects']['index:frontend'];tag='ghcr.io/chronoaiproject/nyxid/frontend:test'
        with patch.dict(os.environ,GITHUB_REF='refs/heads/main'),patch.object(promote.official_release,'authorize'),patch.object(promote.official_release,'manifest'), \
                patch.object(promote.official_release,'verify'),patch.object(promote.official_release,'attach'), \
                patch.object(promote.subprocess,'run') as run,patch.object(promote.subprocess,'check_output',return_value=digest+'\n'):
            promote.promote(plan,evidence,'frontend',tag,Path('regctl'),self.root/'proof',Path('/verifier'))
        self.assertEqual(run.call_args.args[0],['regctl','image','copy','ghcr.io/chronoaiproject/nyxid/frontend@'+digest,tag])
        shutil.rmtree(evidence/'backend')
        with patch.object(promote.subprocess,'run') as run,self.assertRaises(ValueError):promote.promote(plan,evidence,'frontend',tag,Path('regctl'),self.root/'proof',Path('/verifier'))
        run.assert_not_called()

    def test_binary_wizard_and_package_graph_cannot_be_changed_by_rehashing_bundle(self):
        directory=self.root/'cli';r=platform_bundle(directory,'node-agent','amd64')
        image_sbom.verify_bundle(directory)
        child=load_json(directory/'binary.cdx.json');child['components'][0]['name']='unrelated'
        write(directory/'binary.cdx.json',child);r['outputs']=bind_outputs(directory,r['outputs']);write(directory/'image-sbom.json',r)
        with self.assertRaisesRegex(ValueError,'compiled/wizard'):image_sbom.verify_bundle(directory)
        (directory/'wizard-seal.json').unlink()
        with self.assertRaises((ValueError,OSError)):image_sbom.verify_bundle(directory)

    def test_missing_image_sbom_blocks_registry_copy(self):
        spec=importlib.util.spec_from_file_location('publisher',Path(__file__).with_name('publish-scanned-image.py'))
        publisher=importlib.util.module_from_spec(spec);spec.loader.exec_module(publisher)
        with patch.object(publisher,'verify_stage',return_value={'subject':{'digest':'sha256:'+'1'*64}}),patch.object(publisher.subprocess,'run') as run:
            with self.assertRaises((ValueError,OSError)):publisher.publish(self.root,'ghcr.io/fixture/nyxid/frontend',Path('regctl'),self.root/'proof',Path('/verifier'))
        run.assert_not_called()

    def test_final_release_archive_composition_and_every_subject_digest_replay(self):
        from source_artifacts import archives
        plan=cli_plan();artifacts=self.root/'artifacts';artifacts.mkdir();evidence=self.root/'evidence';evidence.mkdir()
        outputs=[]
        for subject in plan['subjects']:
            path=artifacts/subject['name']
            if subject.get('artifact_kind')!='executable-zip':
                path.write_text(subject['name']);continue
            payload=b'actual final binary bytes for '+subject['target'].encode()
            with tarfile.open(path,'w:gz') as tar:
                for asset in subject['assets']:
                    data=payload if asset['path']=='nyxid' else b'license'
                    item=tarfile.TarInfo(asset['path']);item.size=len(data);tar.addfile(item,io.BytesIO(data))
            with tempfile.TemporaryDirectory() as temporary,patch.object(archives,'check_binary_target'):
                inspection=archives.inspect_archive(path,subject,Path(temporary))
            binary=simple('actual binary','sha256:'+hashlib.sha256(payload).hexdigest())
            archive=archives.archive_sbom(binary,subject,inspection)
            for suffix,value in (('.binary.cdx.json',binary),('.cdx.json',archive)):
                name=subject['name']+suffix;write(evidence/name,value);outputs.append(name)
        credential=simple('credential',file_digest(artifacts/'releases.json'))
        write(evidence/'releases.json.cdx.json',credential);outputs.append('releases.json.cdx.json')
        record=dict(schema='nyxid.release-sboms.v1',source=plan['source'],
            subjects={s['name']:file_digest(artifacts/s['name']) for s in plan['subjects']},outputs=bind_outputs(evidence,outputs))
        write(evidence/'release-sboms.json',record)
        with patch.object(archives,'check_binary_target'):self.assertEqual(release_sboms.verify(plan,artifacts,evidence),record)
        name=next(s['name'] for s in plan['subjects'] if s.get('artifact_kind')=='executable-zip')+'.binary.cdx.json'
        d=load_json(evidence/name);d['metadata']['component']['hashes'][0]['content']='f'*64;write(evidence/name,d)
        record['outputs']=bind_outputs(evidence,outputs);write(evidence/'release-sboms.json',record)
        with patch.object(archives,'check_binary_target'),self.assertRaises(ValueError):release_sboms.verify(plan,artifacts,evidence)

    def test_release_digest_closure_requires_every_planned_subject_and_sbom(self):
        plan=cli_plan();artifacts=self.root/'artifacts';artifacts.mkdir();evidence=self.root/'evidence';evidence.mkdir()
        for s in plan['subjects']:(artifacts/s['name']).write_text(s['name'])
        subjects={s['name']:file_digest(artifacts/s['name']) for s in plan['subjects']}
        r=dict(schema='nyxid.release-sboms.v1',source=plan['source'],subjects=subjects,outputs={})
        write(evidence/'release-sboms.json',r)
        with self.assertRaisesRegex(ValueError,'planned'):release_sboms.verify(plan,artifacts,evidence)
        (artifacts/'releases.json').write_text('tampered')
        with self.assertRaisesRegex(ValueError,'digest closure'):release_sboms.verify(plan,artifacts,evidence)


class RealFilesystem(unittest.TestCase):
    def test_real_syft_scans_exact_regctl_oci_subject_and_retains_distribution_package(self):
        # An actual local Docker-format archive needs no Docker daemon. Real
        # regctl import and Syft scan exercise descriptor encoding and conversion.
        syft=Path(os.environ['SYFT_BIN']);regctl=Path(os.environ['REGCTL_BIN'])
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);layer=io.BytesIO()
            files={'etc/os-release':b'ID=debian\nVERSION_ID="12"\nPRETTY_NAME="Debian GNU/Linux 12"\n',
                   'var/lib/dpkg/status':b'Package: fixture\nStatus: install ok installed\nArchitecture: all\nVersion: 1.0\nMaintainer: Fixture <fixture@example.invalid>\nDescription: synthetic package\n\n'}
            with tarfile.open(fileobj=layer,mode='w') as tar:
                for name,content in files.items():
                    member=tarfile.TarInfo(name);member.size=len(content);tar.addfile(member,io.BytesIO(content))
            config=json.dumps(dict(architecture='amd64',os='linux',config={'Labels':{'org.opencontainers.image.revision':COMMIT}},rootfs=dict(type='layers',diff_ids=['sha256:'+hashlib.sha256(layer.getvalue()).hexdigest()]))).encode()
            config_id=hashlib.sha256(config).hexdigest();archive=root/'image.tar'
            with tarfile.open(archive,'w') as tar:
                manifest=json.dumps([dict(Config=config_id+'.json',RepoTags=['fixture:sbom'],Layers=['layer.tar'])]).encode()
                for name,content in ((config_id+'.json',config),('layer.tar',layer.getvalue()),('manifest.json',manifest)):
                    item=tarfile.TarInfo(name);item.size=len(content);tar.addfile(item,io.BytesIO(content))
            local='ocidir://'+str(root/'oci')+':staged'
            subprocess.run([str(regctl),'image','import',local,str(archive)],check=True,capture_output=True)
            digest=subprocess.check_output([str(regctl),'manifest','digest',local],text=True).strip()
            image_sbom.run_syft(syft,['scan','oci-dir:'+str(root/'oci'),'--scope','squashed','-o','syft-json'],root/'raw.json')
            image_sbom.run_syft(syft,['convert',str(root/'raw.json'),'-o','cyclonedx-json@1.6'],root/'cdx.json')
            raw=load_json(root/'raw.json');converted=load_json(root/'cdx.json')
            # Real Syft conversion omits coordinates without metadata, as found
            # for glibc ELF references in the exported node-agent rootfs. Retain
            # the coordinate in the catalog and exercise that exact encoder path.
            raw.setdefault('files', []).append(dict(id='metadata-less-elf-file',
                location=dict(path='/usr/lib/fixture.so', accessPath='/usr/lib/fixture.so')))
            write(root/'raw.json',raw)
            image_sbom.run_syft(syft,['convert',str(root/'raw.json'),'-o','cyclonedx-json@1.6'],root/'cdx.json')
            converted=load_json(root/'cdx.json')
            with self.assertRaisesRegex(ValueError, '/usr/lib/fixture.so'):
                image_sbom.verify_conversion(raw,converted)
            image_sbom.complete_file_inventory(raw,converted)
            receipt=dict(component='frontend',platform='linux/amd64',source_commit=COMMIT,image_id='sha256:'+config_id,subject=dict(digest=digest,mediaType='application/vnd.oci.image.manifest.v1+json'))
            image_sbom.verify_catalog(raw,receipt);image_sbom.verify_conversion(raw,converted)
            result=image_sbom.assembled(receipt,converted,{'assets':simple()})
            self.assertTrue(any(c.get('name')=='fixture' and c.get('version')=='1.0' for c in result['components']))
            validate(result,digest,COMMIT)
            # Exercise the exact attachment command using only a disposable
            # local OCI layout. No registry, release or network write occurs.
            from publish_sbom import attach
            write(root/'image.cdx.json',result)
            subject='ocidir://'+str(root/'oci')+'@'+digest
            attach(regctl,subject,root/'image.cdx.json')
            downloaded=subprocess.check_output([str(regctl),'artifact','get','--subject',subject,
                '--filter-artifact-type','application/vnd.cyclonedx+json'])
            self.assertEqual(downloaded,(root/'image.cdx.json').read_bytes())
            referrers=json.loads(subprocess.check_output([str(regctl),'artifact','list',subject,'--format','{{json .Manifest}}']))
            self.assertEqual(len(referrers['manifests']),1)
            artifact='ocidir://'+str(root/'oci')+'@'+referrers['manifests'][0]['digest']
            attached=json.loads(subprocess.check_output([str(regctl),'manifest','get',artifact,'--format','raw-body']))
            self.assertEqual(attached['subject']['digest'],digest)
            self.assertEqual(attached['layers'][0]['digest'],file_digest(root/'image.cdx.json'))


if __name__=='__main__':unittest.main()
