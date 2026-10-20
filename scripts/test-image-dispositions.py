#!/usr/bin/env python3
"""Exact eligibility, expiry, authenticated security-index and code-absence negatives."""
import copy
from datetime import date, timedelta
import json
import lzma
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import image_dispositions as policy


class Dispositions(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); (self.root / 'security').mkdir()
        self.evidence = self.root / 'evidence'; index_dir = self.evidence / 'security-index'; index_dir.mkdir(parents=True)
        index = lzma.compress(b'Package: linux-libc-dev\nVersion: 1.0\nSource: linux\nArchitecture: amd64\n\n')
        release = ('SHA256:\n '+policy.digest(index)+' '+str(len(index))+' main/binary-amd64/Packages.xz\n').encode()
        (index_dir / 'Packages.xz').write_bytes(index); (index_dir / 'InRelease').write_bytes(release)
        catalog = 'ubuntu\tnoble\tnoble-security\tmain,universe\thttps://snapshot.ubuntu.com/ubuntu/20261020T000000Z\t'+policy.digest(release)+'\n'
        (self.root / 'security/apt-snapshots.tsv').write_text(catalog)
        self.vuln = dict(VulnerabilityID='CVE-2099-1234', PkgName='linux-libc-dev', InstalledVersion='1.0',
                         Severity='CRITICAL', Status='affected', Title='kernel: reviewed kernel API issue')
        self.report = dict(Results=[dict(Class='os-pkgs', Packages=[dict(Name='linux-libc-dev', Version='1.0',
                            Arch='amd64', SrcName='linux', SrcVersion='1.0')], Vulnerabilities=[self.vuln])])
        self.report['Results'][0]['Packages'][0]['ID']='linux-libc-dev@1.0'
        self.report['Results'][0]['Packages'] += [dict(Name='build-essential', ID='build-essential@1.0', DependsOn=['libc6-dev@1.0']),
                                                dict(Name='libc6-dev', ID='libc6-dev@1.0', DependsOn=['linux-libc-dev@1.0'])]
        self.findings = [dict(id=self.vuln['VulnerabilityID'], package='linux-libc-dev', version='1.0', severity='CRITICAL')]
        self.item = dict(cve='CVE-2099-1234', source_package='linux', binary_package='linux-libc-dev', version='1.0',
            source_version='1.0', component='machine-node-only', architecture='amd64', issued_on=date.today().isoformat(),
            expires_on=(date.today()+timedelta(days=30)).isoformat(), owner='fixture reviewer',
            rationale='The affected kernel API is absent: this fixture package contains only non-executable UAPI headers.',
            affected_api='reviewed kernel API', negative_test=policy.KIND, content_sha256='a'*64,
            catalog_sha256=policy.digest(catalog.encode()), finding_sha256=policy.finding_digest(self.vuln))
        self.data = dict(schema='nyxid.image-dispositions.v1', declared_runtime={'machine-node-only':['build-essential','linux-libc-dev']}, dispositions=[self.item])
        self.write()

    def write(self): (self.root / 'security/image-dispositions.json').write_text(json.dumps(self.data))
    def apply(self):
        with patch.object(policy, 'header_content', return_value='a'*64):
            return policy.apply(self.report, self.findings, 'sha256:'+'1'*64, 'machine-node-only',
                                'linux/amd64', self.evidence, root=self.root)

    def test_exact_review_with_authenticated_candidate_and_executable_proof_can_match(self):
        self.assertEqual(self.apply(), [])

    def test_empty_policy_never_suppresses_a_finding(self):
        self.data['dispositions']=[]; self.write(); self.assertEqual(self.apply(),self.findings)

    def test_new_cve_version_component_architecture_and_package_do_not_match(self):
        for key, value in (('cve','CVE-2099-9999'),('version','2.0'),('component','machine-hosted'),('architecture','arm64'),('binary_package','other')):
            original = copy.deepcopy(self.data)
            self.data['dispositions'][0][key]=value; self.write()
            with self.subTest(key=key):
                try: result=self.apply()
                except ValueError: pass
                else: self.assertEqual(result,self.findings)
            self.data=original

    def test_expired_future_long_missing_owner_or_rationale_are_gate_errors(self):
        for key,value in (('expires_on',date.today().isoformat()),('issued_on',(date.today()+timedelta(days=1)).isoformat()),
                          ('expires_on',(date.today()+timedelta(days=31)).isoformat()),('owner',''),('affected_api',''),('rationale','')):
            original=copy.deepcopy(self.data); self.data['dispositions'][0][key]=value; self.write()
            with self.subTest(key=key,value=value),self.assertRaises(ValueError):self.apply()
            self.data=original

    def test_later_snapshot_catalog_content_and_source_changes_fail(self):
        for mode in ('catalog','content','source','source-version','finding-description','new-finding-field'):
            with self.subTest(mode=mode):
                original = copy.deepcopy(self.report)
                if mode=='catalog': (self.root/'security/apt-snapshots.tsv').write_text('changed\n')
                elif mode=='content': self.data['dispositions'][0]['content_sha256']='b'*64; self.write()
                elif mode=='source': self.report['Results'][0]['Packages'][0]['SrcName']='different-source'
                elif mode=='source-version':self.report['Results'][0]['Packages'][0]['SrcVersion']='2.0'
                elif mode=='finding-description':self.vuln['Description']='new unreviewed code path'
                else:self.vuln['NewSecurityField']='changed'
                with self.assertRaises(ValueError):self.apply()
                self.report=original; self.vuln=self.report['Results'][0]['Vulnerabilities'][0]
                self.data['dispositions'][0]['content_sha256']='a'*64; self.write()
                (self.root/'security/apt-snapshots.tsv').write_text('ubuntu\tnoble\tnoble-security\tmain,universe\thttps://snapshot.ubuntu.com/ubuntu/20261020T000000Z\t'+policy.digest((self.evidence/'security-index/InRelease').read_bytes())+'\n')

    def test_available_fixed_version_always_blocks_even_on_renewal(self):
        self.vuln['FixedVersion']='2.0'; self.item['finding_sha256']=policy.finding_digest(self.vuln);self.write()
        with self.assertRaisesRegex(ValueError,'available fix'):self.apply()

    def test_newer_security_archive_candidate_prevents_matching_or_renewal(self):
        with patch.object(policy,'security_candidate',return_value=dict(version='2.0',source_version='2.0')):
            with self.assertRaisesRegex(ValueError,'candidate'):self.apply()

    def test_security_release_and_package_index_tampering_fail(self):
        for name in ('InRelease','Packages.xz'):
            p=self.evidence/'security-index'/name; original=p.read_bytes();p.write_bytes(original+b'tampered')
            with self.subTest(name=name),self.assertRaises(ValueError):self.apply()
            p.write_bytes(original)

    def test_missing_workstation_dependency_chain_and_resolved_records_fail(self):
        for mode in ('compiler', 'libc-development', 'resolved'):
            original=copy.deepcopy(self.report)
            if mode=='compiler': self.report['Results'][0]['Packages'][1]['DependsOn']=[]
            elif mode=='libc-development':self.report['Results'][0]['Packages'][2]['DependsOn']=[]
            else:self.report['Results'][0]['Vulnerabilities']=[]
            with self.subTest(mode=mode),self.assertRaises(ValueError):self.apply()
            self.report=original

    def test_critical_library_executable_and_network_package_cannot_use_header_kind(self):
        for package in ('libc6','openssl','libssl3','curl'):
            self.data['dispositions'][0]['binary_package']=package;self.write()
            with self.subTest(package=package),self.assertRaises(ValueError):self.apply()

    def test_negative_proof_rejects_elf_executable_and_every_non_header_path(self):
        policy.header_member('/usr/include/linux/fixture.h',0o644,b'#define FIXTURE 1\n')
        for path,mode,data in (('/usr/include/linux/fixture.h',0o755,b'script'),('/usr/include/linux/fixture.h',0o644,b'\x7fELFbinary'),
                               ('/usr/bin/fixture',0o644,b'content'),('/usr/share/doc/linux-libc-dev/copyright',0o644,b'license')):
            with self.subTest(path=path,mode=mode),self.assertRaises(ValueError):policy.header_member(path,mode,data)

    def test_host_tar_inspection_checks_the_final_dpkg_file_list_and_actual_bytes(self):
        import io, tarfile
        from contextlib import contextmanager
        @contextmanager
        def container(image): yield 'container', {'Id': image}
        def run_case(mode):
            path='/usr/include/linux/fixture.h'
            names=['/usr','/usr/include','/usr/include/linux',path]
            if mode=='outside':names.append('/usr/bin/bad')
            def extract(container,path,destination):
                destination.mkdir()
                (destination/'linux-libc-dev:amd64.list').write_text('\n'.join(names)+'\n')
            data=io.BytesIO()
            with tarfile.open(fileobj=data,mode='w') as archive:
                for name in names:
                    if mode=='missing' and name==path:continue
                    member=tarfile.TarInfo(name.lstrip('/'))
                    if name in names[:3]:member.type=tarfile.DIRTYPE;member.mode=0o755;archive.addfile(member)
                    else:
                        content=b'\x7fELFcode' if mode=='elf' else b'#define FIXTURE 1\n'
                        member.mode=0o755 if mode=='executable' else 0o644
                        member.size=len(content);archive.addfile(member,io.BytesIO(content))
            class Child:
                def __init__(self,*args,**kwargs):self.stdout=io.BytesIO(data.getvalue())
                def __enter__(self):return self
                def __exit__(self,*args):pass
                def wait(self):return 0
                def terminate(self):pass
            with tempfile.TemporaryDirectory() as temporary,patch.object(policy,'image_container',container), \
                    patch.object(policy,'copy',extract),patch.object(policy.subprocess,'Popen',Child):
                return policy.header_content('sha256:'+'1'*64,'amd64',Path(temporary))
        self.assertRegex(run_case('valid'),r'^[0-9a-f]{64}$')
        for mode in ('outside','missing','elf','executable'):
            with self.subTest(mode=mode),self.assertRaises(ValueError):run_case(mode)

    def test_unknown_kind_missing_evidence_and_duplicate_records_fail(self):
        for mode in ('kind','missing-field','duplicate','missing-evidence'):
            original=copy.deepcopy(self.data)
            if mode=='kind':self.data['dispositions'][0]['negative_test']='arbitrary-script'
            elif mode=='missing-field':del self.data['dispositions'][0]['content_sha256']
            elif mode=='duplicate':self.data['dispositions'].append(copy.deepcopy(self.item))
            self.write()
            with self.subTest(mode=mode),self.assertRaises(ValueError):
                if mode=='missing-evidence':policy.apply(self.report,self.findings,'sha256:'+'1'*64,'machine-node-only','linux/amd64',None,root=self.root)
                else:self.apply()
            self.data=original

if __name__=='__main__':unittest.main()
