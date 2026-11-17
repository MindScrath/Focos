"""陈旧时间戳/未校验 pyc 不得冒充摘要所记录的源码版本。"""
import hashlib
import json
import os
from pathlib import Path
import py_compile
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))


def cached_then_changed(path,old,new,mode=py_compile.PycInvalidationMode.TIMESTAMP):
    assert len(old)==len(new)
    path.write_bytes(old)
    stamp=path.stat()
    cache=path.parent/'__pycache__'/f'{path.stem}.{sys.implementation.cache_tag}.pyc'
    cache.parent.mkdir(exist_ok=True)
    py_compile.compile(str(path),cfile=str(cache),doraise=True,invalidation_mode=mode,optimize=0)
    path.write_bytes(new)
    os.utime(path,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))


class PythonSourceTests(unittest.TestCase):
    def fixture(self):
        directory=tempfile.TemporaryDirectory();self.addCleanup(directory.cleanup)
        root=Path(directory.name)
        shutil.copytree(ROOT/'scripts',root/'scripts',ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (root/'tests').mkdir()
        for name in ['inventory.json','reference.lock.json']:
            shutil.copyfile(ROOT/'tests'/name,root/'tests'/name)
        return root

    @staticmethod
    def environment():
        return {key:value for key,value in os.environ.items() if key not in ('PYTHONPYCACHEPREFIX','PYTHONOPTIMIZE')}

    def poison(self,root,module,mode=py_compile.PycInvalidationMode.TIMESTAMP):
        path=root/'scripts'/module;marker=root/'loaded-version'
        source=path.read_bytes()
        suffix=f'\nfrom pathlib import Path as _ProbePath\n_ProbePath({str(marker)!r}).write_text("cached-old")\n'.encode()
        old=source+suffix;new=source+suffix.replace(b'cached-old',b'source-new')
        cached_then_changed(path,old,new,mode)
        return marker,hashlib.sha256(new).hexdigest()

    def test_all_cli_entrypoints_ignore_existing_project_bytecode(self):
        for entry in ['run_tests.py','ref/build_initramfs.py','ref/replay_evidence.py','fetch_judges.py','sync_test_plan.py']:
            with self.subTest(entry=entry):
                root=self.fixture();marker,_=self.poison(root,'inventory.py')
                result=subprocess.run([sys.executable,'-B',str(root/'scripts'/entry),'--help'],cwd=root,
                                      env=self.environment(),capture_output=True,timeout=15)
                self.assertEqual(result.returncode,0,result.stderr.decode())
                self.assertEqual(marker.read_text(),'source-new')

    def test_successful_runner_records_the_source_that_actually_executed(self):
        modes=[py_compile.PycInvalidationMode.TIMESTAMP,py_compile.PycInvalidationMode.UNCHECKED_HASH]
        for module in ['inventory.py','input_snapshot.py','reference_protocol.py','reference_manifest.py','lmbench_trace.py']:
            for mode in modes:
                with self.subTest(module=module,mode=mode):
                    root=self.fixture();marker,expected=self.poison(root,module,mode)
                    names=json.loads((root/'tests/inventory.json').read_text())['groups']['iperf']['subcases']
                    log=root/'serial.log';log.write_text('#### OS COMP TEST GROUP START iperf-musl ####\n'+
                        ''.join(f'====== iperf {name} end: success ======\n' for name in names)+
                        '#### OS COMP TEST GROUP END iperf-musl ####\n')
                    result=subprocess.run([sys.executable,'-B',str(root/'scripts/run_tests.py'),'--log',str(log),'--out',str(root/'out'),
                                           '--group','iperf','--libc','musl'],cwd=root,env=self.environment(),capture_output=True,timeout=15)
                    self.assertEqual(result.returncode,0,result.stderr.decode())
                    summary=json.loads((root/'out/summary.json').read_text())
                    self.assertTrue(summary['meta']['execution_valid'])
                    self.assertEqual(marker.read_text(),'source-new')
                    self.assertEqual(summary['meta']['source_sha256']['scripts/'+module],expected)

    def test_judge_child_ignores_stale_custom_helper_bytecode(self):
        import run_tests
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);helper=root/'helper.py'
            cached_then_changed(helper,b'def passed(): return 1\n',b'def passed(): return 0\n')
            judge=root/'judge.py';judge.write_text('from helper import passed\nimport json\nprint(json.dumps([{"name":"one","all":1,"pass":passed()}]))\n')
            with patch.dict(os.environ,self.environment(),clear=True):
                result=run_tests.run_judge(judge,'input',10)
            self.assertIsNone(result.error)
            self.assertEqual(result.items[0]['pass'],0)
