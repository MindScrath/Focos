"""冻结输入、区域计划和判分器来源不能独立漂移。"""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'scripts'))
from inventory import load_inventory
from sync_test_plan import project_plan, sync
from fetch_judges import fetch
from ref.replay_evidence import unpack


class InputTests(unittest.TestCase):
    def test_raw_serial_evidence_has_an_external_hash_index(self):
        import re, subprocess
        index=json.loads((ROOT/'tests/results/raw-evidence-2026-11-17.json').read_text())
        self.assertRegex(index['archive_sha256'],r'^[0-9a-f]{64}$')
        self.assertEqual(len(index['files']),4)
        for name,sha in index['files'].items():
            path=Path(name)
            self.assertFalse(path.is_absolute())
            self.assertNotIn('..',path.parts)
            self.assertRegex(sha,r'^[0-9a-f]{64}$')
            self.assertFalse((ROOT/path).exists())
            if 'schema2-lua' in name:
                validation=ROOT/'tests/results/reference-2026-11-16/payload-binding.json'
                key='schema2-lua/serial.log'
            else:
                validation=(ROOT/path).parents[1]/'validation.json';key='lua/serial.log'
            self.assertEqual(json.loads(validation.read_text())['evidence_sha256'][key],sha)
        tracked=subprocess.check_output(['git','ls-files','-z','--','*.log'],cwd=ROOT).split(b'\0')
        for name in tracked:
            path=ROOT/name.decode() if name else None
            if path is not None and path.is_file():
                self.assertIsNone(re.search(rb'[A-Za-z0-9_.+%-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}',path.read_bytes()),str(path))

    def test_inventory_projections_are_current(self):
        sync(check=True)

    def test_plan_projection_preserves_execution_results(self):
        plan = json.loads((ROOT/'tests/regional-plan.json').read_text())
        plan['groups'][0]['status_by_libc']['musl'] = 'FAIL'
        projected = project_plan(plan, load_inventory())
        self.assertEqual(projected['groups'][0]['status_by_libc']['musl'], 'FAIL')
        self.assertEqual(projected['groups'][0]['status_by_libc']['glibc'], 'NOT_RUN')
        self.assertEqual(projected['case_count'], 427)
        self.assertEqual(sum(g['case_count'] for g in projected['groups']), 427)
        self.assertEqual(project_plan(projected, load_inventory()), projected)

    def test_invalid_inventory_is_rejected(self):
        original = load_inventory()
        changes = [lambda d: d.update(total_subcases=426),
                   lambda d: d['groups']['basic']['subcases'].append('test_brk'),
                   lambda d: d['judge']['files'].clear(),
                   lambda d: d['groups']['basic'].update(judge_file='../judge_basic.py')]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'inventory.json'
            for change in changes:
                data = copy.deepcopy(original)
                change(data)
                path.write_text(json.dumps(data))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    load_inventory(path)

    def test_fetch_does_not_overwrite_existing_modified_judge(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)/'official'
            (root/'judge').mkdir(parents=True)
            path = root/'judge/judge_basic.py'
            path.write_text('modified')
            with self.assertRaisesRegex(ValueError, 'modified'):
                fetch(root)
            self.assertEqual(path.read_text(), 'modified')

    def test_evidence_archive_rejects_changed_bytes_and_unsafe_paths(self):
        import importlib.util
        import io
        import tarfile
        import hashlib
        spec = importlib.util.spec_from_file_location('replay', ROOT/'scripts/ref/replay_evidence.py')
        replay = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(replay)
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp)/'bad.tar.gz'
            with tarfile.open(archive, 'w:gz') as target:
                item = tarfile.TarInfo('test-001-evidence-2026-11-08/../../escape')
                item.size = 1
                target.addfile(item, io.BytesIO(b'x'))
            sha = hashlib.sha256(archive.read_bytes()).hexdigest()
            for expected in ['0'*64, sha]:
                with self.subTest(expected=expected), self.assertRaises(ValueError):
                    replay.unpack(archive, Path(tmp)/'out', expected)
            self.assertFalse((Path(tmp)/'escape').exists())

    def evidence_archive(self, root, *, corrupt=False):
        import hashlib,io,tarfile
        archive=root/'evidence.tar.gz';prefix='test-001-evidence-2026-11-08'
        files={f'data/{i}.txt':f'evidence {i}'.encode() for i in range(38)}
        files['SHA256SUMS']=''.join(f'{hashlib.sha256(data).hexdigest()}  {name}\n' for name,data in files.items()).encode()
        if corrupt:files['data/0.txt']=b'corrupt'
        with tarfile.open(archive,'w:gz') as target:
            for name,data in files.items():
                member=tarfile.TarInfo(prefix+'/'+name);member.size=len(data)
                target.addfile(member,io.BytesIO(data))
        return archive,hashlib.sha256(archive.read_bytes()).hexdigest()

    def test_evidence_reuse_rejects_existing_links_without_external_writes(self):
        import os
        prefix='test-001-evidence-2026-11-08'
        for kind in ['destination','ancestor','root','directory','file','checksum','dangling','hardlink']:
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);archive,sha=self.evidence_archive(root);external=root/'external';external.mkdir()
                sentinel=external/'sentinel';sentinel.write_bytes(b'preserve')
                destination=root/'out';target=destination/prefix
                if kind=='destination':destination.symlink_to(external,target_is_directory=True)
                elif kind=='ancestor':
                    parent=root/'alias';parent.symlink_to(external,target_is_directory=True);destination=parent/'out'
                elif kind=='root':
                    destination.mkdir();target.symlink_to(external,target_is_directory=True)
                elif kind=='directory':
                    target.mkdir(parents=True);(target/'data').symlink_to(external,target_is_directory=True)
                else:
                    (target/'data').mkdir(parents=True)
                    name=target/'SHA256SUMS' if kind=='checksum' else target/'data/0.txt'
                    if kind=='hardlink':os.link(sentinel,name)
                    else:name.symlink_to(external/'missing' if kind=='dangling' else sentinel)
                before={p.name:p.read_bytes() for p in external.iterdir() if p.is_file()}
                with self.assertRaises(ValueError):unpack(archive,destination,sha)
                self.assertEqual({p.name:p.read_bytes() for p in external.iterdir() if p.is_file()},before)

    def test_evidence_staging_validates_before_publish_and_reuses_only_exact_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);archive,sha=self.evidence_archive(root);destination=root/'out'
            extracted=unpack(archive,destination,sha)
            self.assertEqual(unpack(archive,destination,sha),extracted)
            original=(extracted/'data/0.txt').read_bytes()
            (extracted/'data/0.txt').write_bytes(b'modified evidence')
            with self.assertRaises(ValueError):unpack(archive,destination,sha)
            self.assertEqual((extracted/'data/0.txt').read_bytes(),b'modified evidence')
            bad,sha=self.evidence_archive(root,corrupt=True)
            fresh=root/'fresh'
            with self.assertRaises(ValueError):unpack(bad,fresh,sha)
            self.assertFalse(fresh.exists())

    def test_replay_rejects_output_links_and_report_archive_alias_before_external_commands(self):
        from unittest.mock import patch
        from ref import replay_evidence as replay
        for kind in ['out','report','cache','archive-alias','current-input']:
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);out=root/'out';external=root/'external';external.mkdir()
                sentinel=external/'sentinel';sentinel.write_bytes(b'preserve')
                if kind=='out':out.symlink_to(external,target_is_directory=True)
                else:
                    out.mkdir()
                    if kind=='report':(out/'replay.json').symlink_to(sentinel)
                    elif kind=='cache':
                        asset=json.loads(replay.REFERENCE_LOCK.read_text())['evidence']['asset']
                        (out/asset).symlink_to(external/'missing')
                    elif kind=='archive-alias':
                        import os
                        os.link(sentinel,out/'replay.json')
                    else:
                        (out/'current').mkdir();(out/'current/input.tar.gz').write_bytes(b'preserve')
                source=out/'current/input.tar.gz' if kind=='current-input' else sentinel
                args=['--out',str(out)]+(['--archive',str(source)] if kind in ('archive-alias','current-input') else [])
                with patch.object(replay,'unpack',side_effect=AssertionError('unpack executed')),patch.object(replay.subprocess,'run',side_effect=AssertionError('external command executed')),self.assertRaises(ValueError):
                    replay.main(args)
                self.assertEqual(sentinel.read_bytes(),b'preserve')

    def test_replay_report_atomically_detaches_unrelated_hardlink(self):
        import hashlib,os,subprocess
        from unittest.mock import patch
        from ref import replay_evidence as replay
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);out=root/'out';out.mkdir();archive=root/'archive';archive.write_bytes(b'input')
            lock=root/'lock.json';lock.write_text(json.dumps({'evidence':{'asset':'archive','sha256':hashlib.sha256(b'input').hexdigest(),'historical_runner_commit':'fixed'}}))
            updates=root/'updates.json';updates.write_text(json.dumps({'schema_version':1,'archive_sha256':hashlib.sha256(b'input').hexdigest(),'updates':[]}))
            source=root/'original';(source/'results').mkdir(parents=True);(source/'logs').mkdir()
            original={'meta':{'log':'sample0.log','libc':'musl'},'groups':[{'group':'iperf'}],'totals':{'pass':0}}
            for i in range(15):
                path=source/'results'/str(i);path.mkdir();(path/'summary.json').write_text(json.dumps(original))
            for i in range(7):(source/'logs'/f'sample{i}.log').write_text('evidence')
            sentinel=root/'sentinel';sentinel.write_bytes(b'preserve');os.link(sentinel,out/'replay.json')
            def run(cmd,**kwargs):
                dest=Path(cmd[cmd.index('--out')+1]);dest.mkdir(parents=True,exist_ok=True)
                data=dict(original);data['meta']={'environment':{'status':'LEGACY_UNVERIFIED'}}
                (dest/'summary.json').write_text(json.dumps(data))
                return subprocess.CompletedProcess(cmd,1,stdout='',stderr='')
            import contextlib,io
            with patch.object(replay,'REFERENCE_LOCK',lock),patch.object(replay,'REPLAY_UPDATES',updates),patch.object(replay,'unpack',return_value=source),patch.object(replay.subprocess,'run',side_effect=run),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(replay.main(['--archive',str(archive),'--out',str(out)]),0)
            self.assertEqual(sentinel.read_bytes(),b'preserve')
            self.assertFalse(sentinel.samefile(out/'replay.json'))
            self.assertTrue(json.loads((out/'replay.json').read_text())['all_match'])
            serial=out/'current/0/serial.log';os.link(archive,serial)
            with patch.object(replay,'REFERENCE_LOCK',lock),patch.object(replay,'REPLAY_UPDATES',updates),patch.object(replay,'unpack',return_value=source),patch.object(replay.subprocess,'run',side_effect=AssertionError('input archive may be overwritten')),self.assertRaisesRegex(ValueError,'aliases'):
                replay.main(['--archive',str(archive),'--out',str(out)])
            self.assertEqual(archive.read_bytes(),b'input');serial.unlink()
            # 子进程退出 1 却没写新摘要时，不能接受复用目录的旧成功结果。
            def crashed(cmd,**kwargs):return subprocess.CompletedProcess(cmd,1,stdout='',stderr='runner crashed')
            with patch.object(replay,'REFERENCE_LOCK',lock),patch.object(replay,'REPLAY_UPDATES',updates),patch.object(replay,'unpack',return_value=source),patch.object(replay.subprocess,'run',side_effect=crashed),self.assertRaisesRegex(ValueError,'replay failed'):
                replay.main(['--archive',str(archive),'--out',str(out)])

    def test_audited_replay_updates_allow_only_exact_unknown_name_annotations(self):
        from ref.replay_evidence import expected_groups,load_updates
        sha='a'*64
        old=[{'group':'busybox','status':'FAIL','unexpected_in_log':[],'subcases':[{'pass':0,'score':0}]}]
        update={'sample':'sample','group':'busybox','field':'unexpected_in_log','before':[],
                'after':['busybox new command'],'reason':'原始 testcase 行新增诊断'}
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'updates.json'
            def save(item):path.write_text(json.dumps({'schema_version':1,'archive_sha256':sha,'updates':[item]}))
            save(update);updates=load_updates(path.read_bytes(),sha)
            expected=expected_groups('sample',old,updates)
            self.assertEqual(old[0]['unexpected_in_log'],[])
            self.assertEqual(expected[0]['unexpected_in_log'],update['after'])
            self.assertEqual(expected[0]['subcases'],old[0]['subcases'])
            for mutation in [lambda d:d[0].update(status='PASS'),lambda d:d[0]['subcases'][0].update({'pass':1}),
                             lambda d:d[0]['unexpected_in_log'].append('unreviewed name')]:
                current=copy.deepcopy(expected);mutation(current);self.assertNotEqual(current,expected)
            for field in ['status','subcases','extras','score']:
                bad=dict(update,field=field);save(bad)
                with self.assertRaises(ValueError):load_updates(path.read_bytes(),sha)
            save(update)
            with self.assertRaises(ValueError):load_updates(path.read_bytes(),'b'*64)
            changed=copy.deepcopy(old);changed[0]['unexpected_in_log']=['already different']
            with self.assertRaises(ValueError):expected_groups('sample',changed,updates)

    def test_replay_source_set_includes_all_dependencies_and_nested_judges(self):
        from ref import replay_evidence as replay
        with tempfile.TemporaryDirectory() as tmp:
            judges=Path(tmp);(judges/'pkg').mkdir();helper=judges/'pkg/helper.py';helper.write_text('x=1')
            paths=replay.execution_sources(judges)
            self.assertIn(helper,paths)
            for name in ['input_snapshot.py','reference_protocol.py','reference_manifest.py','lmbench_trace.py','inventory.py']:
                self.assertIn(ROOT/'scripts'/name,paths)
            self.assertIn(ROOT/'scripts/judge_team/common.py',paths)
            before={p:replay.digest(p) for p in paths}
            helper.write_text('x=2')
            with self.assertRaises(ValueError):replay.verify_sources(judges,before)
            helper.write_text('x=1');(judges/'new.py').write_text('x=1')
            with self.assertRaises(ValueError):replay.verify_sources(judges,before)

    def test_official_cache_rejects_extra_dependencies(self):
        import shutil
        source=ROOT/'third_party/oskernel-testsuits-cooperation/judge'
        for name in ['json.py','json/__init__.py','extra.txt']:
            with self.subTest(name=name),tempfile.TemporaryDirectory() as tmp:
                dest=Path(tmp)/'cache';shutil.copytree(source,dest/'judge')
                extra=dest/'judge'/name;extra.parent.mkdir(exist_ok=True);extra.write_text('unfixed')
                with self.assertRaises(ValueError):fetch(dest)
                import run_tests,contextlib,io
                from unittest.mock import patch
                with patch.object(run_tests,'capture',side_effect=AssertionError('capture executed')),contextlib.redirect_stderr(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(ROOT/'scripts/run_tests.py'),
                        '--out',str(Path(tmp)/'out'),'--libc','musl','--group','basic','--judge-dir',str(dest/'judge')])
                self.assertEqual(code,2)
