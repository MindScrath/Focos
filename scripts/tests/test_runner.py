#!/usr/bin/env python3
"""Negative and contract tests for scripts/run_tests.py.

Run from the repository root:
    python -m unittest discover -s scripts/tests -v
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'scripts'))
RUNNER = ROOT / "scripts" / "run_tests.py"
INVENTORY = ROOT / "tests" / "inventory.json"
JUDGE_DIR = Path(os.environ.get("TEST001_JUDGE_DIR", ROOT / "third_party" / "oskernel-testsuits-cooperation" / "judge"))
TEAM_DIR = ROOT / "scripts" / "judge_team"

BASIC_OK = """#### OS COMP TEST GROUP START basic-musl ####
========== START test_brk ==========
Before alloc,heap pos: 0
After alloc,heap pos: 64
Alloc again,heap pos: 128
========== END test_brk ==========
========== START test_chdir ==========
chdir ret: 0
  current working dir : /test_chdir/
========== END test_chdir ==========
#### OS COMP TEST GROUP END basic-musl ####
"""

IPERF_OK = """#### OS COMP TEST GROUP START iperf-musl ####
====== iperf BASIC_UDP end: success ======
====== iperf BASIC_TCP end: fail ======
#### OS COMP TEST GROUP END iperf-musl ####
"""


def run(log_text: str, groups: list[str], extra: list[str] | None = None) -> tuple[int, dict]:
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        log = tmp_path / "serial.log"
        log.write_text(log_text, encoding="utf-8")
        out = tmp_path / "out"
        cmd = [
            sys.executable,
            str(RUNNER),
            "--log",
            str(log),
            "--out",
            str(out),
            "--judge-dir",
            str(JUDGE_DIR),
            "--team-judge-dir",
            str(TEAM_DIR),
            "--inventory",
            str(INVENTORY),
            "--libc",
            "musl",
        ]
        for group in groups:
            cmd += ["--group", group]
        if extra:
            cmd += extra
        proc = subprocess.run(cmd, text=True, capture_output=True)
        summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
        return proc.returncode, summary


class JudgeSourceRaceTests(unittest.TestCase):
    def test_freeze_rejects_bytes_that_do_not_match_recorded_hash(self):
        import run_tests
        from inventory import digest
        with tempfile.TemporaryDirectory() as tmp:
            script=Path(tmp)/'judge.py';script.write_text('print("old")\n')
            expected={str(script):digest(script)}
            script.write_text('print("new")\n')
            with self.assertRaisesRegex(ValueError,'changed while freezing'):
                run_tests.freeze_judge_sources({script},expected)

    def test_directory_switch_during_child_open_cannot_replace_executed_source(self):
        import contextlib, io, run_tests
        from unittest.mock import patch
        from inventory import load_inventory, digest
        names=load_inventory()['groups']['iperf']['subcases']
        for dependency in ['script','helper','package']:
            with self.subTest(dependency=dependency), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);original=root/'original';other=root/'other'
                original.mkdir();other.mkdir();alias=root/'alias';alias.symlink_to(original,target_is_directory=True)
                rows=lambda passed: repr([{'name':name,'all':1,'pass':passed,'score':passed} for name in names])
                for directory,passed in [(original,0),(other,1)]:
                    if dependency=='script':
                        (directory/'judge_iperf.py').write_text('import json\nprint(json.dumps('+rows(passed)+'))\n')
                    else:
                        helper=directory/'helper.py';statement='from helper import rows'
                        if dependency=='package':
                            (directory/'pkg').mkdir();(directory/'pkg/__init__.py').write_text('')
                            helper=directory/'pkg/helper.py';statement='from pkg.helper import rows'
                        helper.write_text('rows = '+rows(passed)+'\n')
                        (directory/'judge_iperf.py').write_text(statement+'\nimport json\nprint(json.dumps(rows))\n')
                log=root/'input.log';log.write_text('#### OS COMP TEST GROUP START iperf-musl ####\n#### OS COMP TEST GROUP END iperf-musl ####\n')
                expected=digest(alias/'judge_iperf.py');real_run=subprocess.run
                def launch(*args,**kwargs):
                    alias.unlink();alias.symlink_to(other,target_is_directory=True)
                    try: return real_run(*args,**kwargs)
                    finally:
                        alias.unlink();alias.symlink_to(original,target_is_directory=True)
                with patch.object(run_tests.subprocess,'run',side_effect=launch),contextlib.redirect_stdout(io.StringIO()):
                    code=run_tests.main(['--log',str(log),'--out',str(root/'out'),'--libc','musl','--group','iperf',
                                         '--inventory',str(INVENTORY),'--team-judge-dir',str(alias)])
                data=json.loads((root/'out/summary.json').read_text())
                self.assertEqual(code,1)
                self.assertTrue(data['meta']['execution_valid'])
                self.assertEqual(data['totals']['pass'],0)
                self.assertEqual(data['meta']['judge_sha256'][str(alias/'judge_iperf.py')],expected)
                if dependency=='package':
                    self.assertIn(str(alias/'pkg/helper.py'),data['meta']['judge_sha256'])

    def test_lmbench_unknown_trace_is_normalized_without_changing_known_denominator(self):
        import run_tests
        from inventory import load_inventory
        from unittest.mock import patch
        cfg=load_inventory()['groups']['lmbench'];names=cfg['subcases']
        rows=[{'name':name,'all':1,'pass':1,'score':1} for name in names]
        for trace in ['+ ./lmbench_all lat_new -P 1',"+ ./lmbench_all 'lat_new'  -P  1"]:
            with self.subTest(trace=trace),patch.object(run_tests,'run_judge',return_value=run_tests.JudgeResult(rows,'ok')):
                result=run_tests.judge_group('lmbench',run_tests.Section('lmbench','musl',[trace]),cfg,Path('.'),TEAM_DIR,5)
            self.assertEqual(result['unexpected_in_log'],['lmbench lat_new -P 1'])
            self.assertEqual(len(result['subcases']),24)
            self.assertEqual(result['status'],'PASS')

    def test_lmbench_frozen_traces_and_malformed_commands(self):
        import shlex
        from lmbench_trace import COMMANDS, COMMAND_INDEX, logged_names
        from inventory import load_inventory
        names=load_inventory()['groups']['lmbench']['subcases']
        self.assertEqual(len(COMMANDS),24)
        self.assertEqual(COMMAND_INDEX[('lat_fs','/var/tmp')],18)
        self.assertEqual(COMMAND_INDEX[('bw_pipe','-P','1')],17)
        for index,command in enumerate(COMMANDS):
            self.assertEqual(logged_names('+ ./lmbench_all '+shlex.join(command),names),{names[index]})
        self.assertEqual(logged_names('+ ./busybox echo ok',names),set())
        self.assertEqual(logged_names('+ ./lmbench_all lat_mmap -P 2 512k /var/tmp/XXX',names),
                         {'lmbench lat_mmap -P 2 512k /var/tmp/XXX'})
        self.assertTrue(next(iter(logged_names("+ ./lmbench_all 'broken",names))).startswith('lmbench malformed-trace '))


class RunnerContractTests(unittest.TestCase):
    def test_empty_log_marks_all_not_run(self):
        code, summary = run("", ["basic", "iperf"])
        self.assertEqual(code, 1)
        self.assertEqual([g["status"] for g in summary["groups"]], ["NOT_RUN", "NOT_RUN"])
        self.assertEqual(summary["totals"]["not_run"], 32 + 6)

    def test_missing_group_is_not_run_not_pass(self):
        code, summary = run(BASIC_OK, ["basic", "unixbench"])
        basic, unixbench = summary["groups"]
        self.assertEqual(basic["status"], "PARTIAL")
        self.assertEqual(unixbench["status"], "NOT_RUN")
        self.assertEqual(unixbench["subcases"][0]["status"], "NOT_RUN")
        self.assertEqual(code, 1)

    def test_start_without_end_is_timeout(self):
        text = "#### OS COMP TEST GROUP START lmbench-musl ####\nnull: 0.1\n"
        code, summary = run(text, ["lmbench"])
        self.assertEqual(summary["groups"][0]["status"], "TIMEOUT")
        self.assertEqual(code, 1)

    def test_unknown_subcase_is_reported(self):
        text = BASIC_OK.replace(
            "#### OS COMP TEST GROUP END basic-musl ####",
            "========== START test_unknown ==========\nx\n========== END test_unknown ==========\n"
            "#### OS COMP TEST GROUP END basic-musl ####",
        )
        _, summary = run(text, ["basic"])
        self.assertEqual(summary["groups"][0]["unexpected_in_log"], ["test_unknown"])

    def test_judge_crash_is_judge_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "judge_basic.py"
            bad.write_text("raise SystemExit(3)\n", encoding="utf-8")
            with tempfile.TemporaryDirectory() as out_tmp:
                log = Path(out_tmp) / "serial.log"
                log.write_text(BASIC_OK, encoding="utf-8")
                out = Path(out_tmp) / "out"
                proc = subprocess.run(
                    [
                        sys.executable,
                        str(RUNNER),
                        "--log",
                        str(log),
                        "--out",
                        str(out),
                        "--judge-dir",
                        tmp,
                        "--team-judge-dir",
                        str(TEAM_DIR),
                        "--inventory",
                        str(INVENTORY),
                        "--libc",
                        "musl",
                        "--group",
                        "basic",
                    ],
                    text=True,
                    capture_output=True,
                )
                summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
                self.assertEqual(summary["groups"][0]["status"], "JUDGE_ERROR")
                self.assertEqual(summary["groups"][0]["subcases"][0]["status"], "JUDGE_ERROR")
                self.assertEqual(proc.returncode, 1)

    def test_partial_group_is_not_pass(self):
        text = BASIC_OK.replace(
            "========== START test_brk ==========",
            "========== START test_brk ==========",
        ).replace("After alloc,heap pos: 64", "After alloc,heap pos: 65")
        _, summary = run(text, ["basic"])
        self.assertEqual(summary["groups"][0]["status"], "PARTIAL")
        self.assertEqual(summary["groups"][0]["subcases"][0]["status"], "PARTIAL")

    def test_iperf_team_judge(self):
        _, summary = run(IPERF_OK, ["iperf"])
        group = summary["groups"][0]
        self.assertEqual(group["status"], "PARTIAL")
        self.assertEqual(group["subcases"][0]["status"], "PASS")
        self.assertEqual(group["subcases"][1]["status"], "FAIL")

    def test_capture_timeout_without_output_is_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            cmd = f'"{sys.executable}" -c "import time; time.sleep(3)"'
            proc = subprocess.run(
                [
                    sys.executable,
                    str(RUNNER),
                    "--capture-cmd",
                    cmd,
                    "--capture-artifact", str(RUNNER),
                    "--timeout",
                    "1",
                    "--out",
                    str(out),
                    "--judge-dir",
                    str(JUDGE_DIR),
                    "--team-judge-dir",
                    str(TEAM_DIR),
                    "--inventory",
                    str(INVENTORY),
                    "--libc",
                    "musl",
                    "--group",
                    "basic",
                ],
                text=True,
                capture_output=True,
            )
            summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            self.assertTrue(summary["meta"]["capture_truncated"])
            self.assertEqual(summary["groups"][0]["status"], "TIMEOUT")
            self.assertEqual(summary["groups"][0]["subcases"][0]["status"], "TIMEOUT")
            self.assertEqual(proc.returncode, 1)

    def test_iozone_performance_mode(self):
        text = """#### OS COMP TEST GROUP START iozone-musl ####
iozone throughput write/read measurements
\tChildren see throughput for  4 initial writers \t=   13545.93 kB/sec
\tMax throughput per process \t\t\t=    3524.04 kB/sec
#### OS COMP TEST GROUP END iozone-musl ####
"""
        _, summary = run(text, ["iozone"])
        group = summary["groups"][0]
        self.assertEqual(group["subcases"][0]["status"], "PASS")



class RunnerSafetyTests(unittest.TestCase):
    def invoke(self, data=b"", extra=(), capture=None):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        log = root / 'input.log'
        if data.startswith(b'REF_PROTOCOL 1'):
            import hashlib
            sys.path.insert(0, str(ROOT/'scripts'))
            import gzip
            from reference_manifest import build_identity, initramfs_payload_sha256
            from ref.build_initramfs import archive_bytes
            from inventory import load_inventory, REFERENCE_LOCK, digest
            inventory = load_inventory()
            lock = json.loads(REFERENCE_LOCK.read_text())
            image = root/'disk.img'
            image.write_bytes(b'synthetic disk')
            inventory['image']['uncompressed_sha256'] = digest(image)
            inventory_path = root/'inventory.json'
            inventory_path.write_text(json.dumps(inventory))
            kernel, initramfs = root/'Image', root/'initramfs.gz'
            kernel.write_bytes(b'synthetic kernel')
            placeholder='0'*64
            init=b'#!/busybox sh\n/busybox echo REF_BUILD_ID '+placeholder.encode()+b'\n'
            files={'init':(0o100755,init),'busybox':(0o100755,b'synthetic busybox')}
            manifest = {'schema_version': 2, 'protocol': 1, 'inputs_pinned': True,
                        'builder_sha256':digest(ROOT/'scripts/ref/build_initramfs.py'),
                        'inventory_code_sha256':digest(ROOT/'scripts/inventory.py'),
                        'manifest_code_sha256':digest(ROOT/'scripts/reference_manifest.py'),
                        'busybox_sha256':hashlib.sha256(b'synthetic busybox').hexdigest(),
                        'inputs': {'image': inventory['image']['uncompressed_sha256'],
                                   'kernel_apk': lock['kernel']['sha256']},
                        'inventory_sha256': digest(inventory_path), 'reference_lock_sha256': digest(REFERENCE_LOCK),
                        'selection': {'libcs': ['musl'], 'scripts': ['iperf_testcode.sh'],
                                      'release': lock['kernel']['release']},
                        'content': {'kernel_sha256':digest(kernel),'initramfs_payload_sha256':initramfs_payload_sha256(archive_bytes(files),placeholder)}}
            manifest['build_id'] = build_identity(manifest)
            files['init']=(0o100755,init.replace(placeholder.encode(),manifest['build_id'].encode()))
            initramfs.write_bytes(gzip.compress(archive_bytes(files),mtime=0))
            manifest['outputs']={'kernel_sha256':digest(kernel),'initramfs_sha256':digest(initramfs)}
            path = root/'build.manifest.json'
            path.write_text(json.dumps(manifest))
            newline = b'\r\n' if data.startswith(b'REF_PROTOCOL 1\r\n') else b'\n'
            data = data.replace(b'REF_PROTOCOL 1'+newline, b'REF_PROTOCOL 1'+newline+f"REF_BUILD_ID {manifest['build_id']}".encode()+newline, 1)
            extra = list(extra) + ['--inventory', str(inventory_path), '--reference-image', str(image), '--reference-manifest', str(path), '--reference-artifacts', str(kernel), str(initramfs)]
        log.write_bytes(data)
        out = root / 'out'
        cmd = [sys.executable, '-B', str(RUNNER), '--out', str(out),
               '--libc', 'musl', '--group', 'iperf', '--inventory', str(INVENTORY),
               '--team-judge-dir', str(TEAM_DIR)]
        cmd += ['--capture-cmd', capture, '--capture-artifact', str(RUNNER)] if capture else ['--log', str(log)]
        proc = subprocess.run(cmd + list(extra), capture_output=True, timeout=8)
        payload = json.loads((out / 'summary.json').read_text()) if (out / 'summary.json').exists() else None
        return proc, payload, out

    @staticmethod
    def complete():
        names = ['BASIC_UDP', 'BASIC_TCP', 'PARALLEL_UDP', 'PARALLEL_TCP', 'REVERSE_UDP', 'REVERSE_TCP']
        return ('#### OS COMP TEST GROUP START iperf-musl ####\n' +
                ''.join(f'====== iperf {n} end: success ======\n' for n in names) +
                '#### OS COMP TEST GROUP END iperf-musl ####\n').encode()

    def test_raw_bytes_are_preserved(self):
        raw = b'\xff\x00\r\n' + self.complete()
        proc, data, out = self.invoke(raw)
        self.assertEqual(proc.returncode, 0)
        self.assertEqual((out / 'serial.log').read_bytes(), raw)
        self.assertNotIn('score', data['totals'])
        self.assertIn('log_sha256', data['meta'])

    def test_environment_failure_invalidates_success(self):
        proc, data, _ = self.invoke(b'REF_FAIL network-dummy\n' + self.complete())
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')

    def test_duplicate_nested_and_mismatched_sections_are_errors(self):
        valid = self.complete()
        malformed = [valid + valid,
                     valid.replace(b'END iperf-musl', b'END iperf-glibc'),
                     valid.replace(b'====== iperf BASIC_UDP', b'#### OS COMP TEST GROUP START lua-musl ####\n====== iperf BASIC_UDP')]
        for raw in malformed:
            with self.subTest(raw=raw):
                proc, data, _ = self.invoke(raw)
                self.assertEqual(proc.returncode, 1)
                self.assertEqual(data['groups'][0]['status'], 'LOG_ERROR')

    def test_nonzero_capture_exit_overrides_complete_results(self):
        import shlex
        source = f'import sys;sys.stdout.buffer.write({self.complete()!r});sys.exit(7)'
        proc, data, _ = self.invoke(capture=shlex.join([sys.executable, '-c', source]))
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'CAPTURE_ERROR')

    def test_nonzero_capture_with_unfinished_section_remains_timeout(self):
        import shlex
        raw=self.complete().replace(b'#### OS COMP TEST GROUP END iperf-musl ####\n',b'')
        source=f'import sys;sys.stdout.buffer.write({raw!r});sys.exit(7)'
        proc,data,_=self.invoke(capture=shlex.join([sys.executable,'-c',source]))
        self.assertEqual(proc.returncode,1)
        self.assertEqual(data['meta']['capture_exit_code'],7)
        self.assertEqual(data['groups'][0]['status'],'TIMEOUT')
        self.assertEqual(data['totals']['pass'],0)
        self.assertFalse(data['meta']['execution_valid'])

    def test_manifest_without_mode_requires_reference_protocol(self):
        proc,data,_=self.invoke(self.reference())
        self.assertEqual(proc.returncode,0)
        self.assertIsNotNone(data['meta']['reference_provenance'])
        proc,data,_=self.invoke(self.reference(),['--reference','legacy'])
        self.assertEqual(proc.returncode,2)
        # invoke supplies a real manifest for explicit strict; reuse its artifacts.
        _,_,out=self.invoke(self.reference(),['--reference','strict'])
        root=out.parent
        raw=root/'plain.log';raw.write_bytes(self.complete())
        import run_tests,contextlib,io
        with contextlib.redirect_stdout(io.StringIO()):
            code=run_tests.main(['--log',str(raw),'--out',str(root/'replayed'),'--group','iperf','--libc','musl',
                '--inventory',str(root/'inventory.json'),'--reference-manifest',str(root/'build.manifest.json'),
                '--reference-artifacts',str(root/'Image'),str(root/'initramfs.gz'),'--reference-image',str(root/'disk.img')])
        self.assertEqual(code,1)
        result=json.loads((root/'replayed/summary.json').read_text())
        self.assertFalse(result['meta']['execution_valid'])

    def test_output_hardlinks_preserve_external_archive(self):
        import run_tests,contextlib,io
        from unittest.mock import patch
        for name in ['serial.log','summary.json','summary.md']:
            with self.subTest(name=name),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);out=root/'out';out.mkdir();archive=root/'archive';archive.write_bytes(b'old evidence')
                os.link(archive,out/name)
                with patch.object(run_tests,'capture',return_value=(self.complete(),False,0)),contextlib.redirect_stdout(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(RUNNER),'--out',str(out),'--libc','musl','--group','iperf'])
                self.assertEqual(code,0)
                self.assertEqual(archive.read_bytes(),b'old evidence')
                self.assertFalse(archive.samefile(out/name))

    def test_empty_source_arguments_are_rejected_before_execution(self):
        import contextlib,io,run_tests
        from unittest.mock import patch
        for source in [['--capture-cmd',''],['--capture-cmd',' \n'],['--log','']]:
            with self.subTest(source=source),tempfile.TemporaryDirectory() as tmp,patch.object(run_tests,'capture',side_effect=AssertionError('empty source was executed')),contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(run_tests.main(source+['--capture-artifact',str(RUNNER),'--out',tmp,'--libc','musl','--group','iperf']),2)

    def test_execution_status_combinations_follow_one_precedence_table(self):
        import contextlib,io,run_tests
        from inventory import load_inventory
        from unittest.mock import patch
        complete=self.complete();partial=complete.replace(b'#### OS COMP TEST GROUP END iperf-musl ####\n',b'')
        malformed=complete.replace(b'END iperf-musl',b'END iperf-glibc')
        cases=[
            ('valid',complete,False,0,False,'PASS'),
            ('complete-nonzero',complete,False,7,False,'CAPTURE_ERROR'),
            ('partial-nonzero',partial,False,7,False,'TIMEOUT'),
            ('partial-zero',partial,False,0,False,'TIMEOUT'),
            ('missing-nonzero',b'',False,7,False,'CAPTURE_ERROR'),
            ('log-and-exit',malformed,False,7,False,'LOG_ERROR'),
            ('environment-and-partial',b'REF_FAIL mount-ext4\n'+partial,False,7,False,'ENV_ERROR'),
            ('timeout-after-complete',complete,True,-9,False,'TIMEOUT'),
            ('environment-and-timeout',b'REF_PROTOCOL 999\n'+partial,True,-9,False,'ENV_ERROR'),
            ('partial-and-judge-and-exit',partial,False,7,True,'TIMEOUT'),
            ('complete-and-judge-and-exit',complete,False,7,True,'CAPTURE_ERROR'),
            ('complete-and-judge',complete,False,0,True,'JUDGE_ERROR'),
            ('partial-and-judge',partial,False,0,True,'TIMEOUT'),
        ]
        names=load_inventory()['groups']['iperf']['subcases']
        for name,raw,timed_out,rc,broken,expected in cases:
            with self.subTest(case=name),tempfile.TemporaryDirectory() as tmp:
                judged=run_tests.JudgeResult([], 'error','judge unavailable') if broken else run_tests.JudgeResult(
                    [{'name':n,'all':1,'pass':1,'score':1} for n in names],'ok')
                with patch.object(run_tests,'capture',return_value=(raw,timed_out,rc)),patch.object(run_tests,'run_judge',return_value=judged),contextlib.redirect_stdout(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(RUNNER),'--out',tmp,'--libc','musl','--group','iperf'])
                data=json.loads((Path(tmp)/'summary.json').read_text())
                self.assertEqual(data['groups'][0]['status'],expected)
                self.assertEqual(data['meta']['capture_exit_code'],rc)
                self.assertEqual(code,0 if expected=='PASS' else 1)
                self.assertEqual(data['totals']['pass'],len(names) if expected=='PASS' else 0)
                if broken:self.assertEqual(data['groups'][0]['judge_error'],'judge unavailable')

    def test_complete_and_incomplete_groups_keep_distinct_capture_failure_statuses(self):
        import contextlib,io,run_tests
        from unittest.mock import patch
        raw=self.complete()+b'#### OS COMP TEST GROUP START netperf-musl ####\n'
        with tempfile.TemporaryDirectory() as tmp,patch.object(run_tests,'capture',return_value=(raw,False,7)),contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(run_tests.main(['--capture-cmd','fake','--capture-artifact',str(RUNNER),'--out',tmp,'--libc','musl',
                                            '--group','iperf','--group','netperf']),1)
            data=json.loads((Path(tmp)/'summary.json').read_text())
            self.assertEqual([g['status'] for g in data['groups']],['CAPTURE_ERROR','TIMEOUT'])
            self.assertEqual(data['totals']['pass'],0)

    def test_higher_priority_evidence_dominates_lower_priority_combinations(self):
        import itertools,run_tests
        for status,log_error,exit_code,script_exit in itertools.product(
                ['PASS','PARTIAL','FAIL','NOT_RUN','JUDGE_ERROR','TIMEOUT'],[None,'malformed'],[None,0,7],[0,3]):
            result={'status':status,'section_truncated':False}
            kwargs={'log_error':log_error,'capture_exit':exit_code,'script_exit':script_exit}
            with self.subTest(status=status,log=log_error,exit=exit_code,script=script_exit):
                self.assertEqual(run_tests.execution_status(result,environment_error=True,capture_timeout=True,**kwargs),'ENV_ERROR')
                self.assertEqual(run_tests.execution_status(result,environment_error=True,capture_timeout=False,**kwargs),'ENV_ERROR')
                self.assertEqual(run_tests.execution_status(result,environment_error=False,capture_timeout=True,**kwargs),'TIMEOUT')
                result['section_truncated']=True
                self.assertEqual(run_tests.execution_status(result,environment_error=False,capture_timeout=False,**kwargs),'TIMEOUT')

    def test_strict_incomplete_prefix_preserves_timeout_without_hiding_protocol_faults(self):
        import contextlib,io,run_tests,shlex
        from inventory import digest
        from unittest.mock import patch
        _,_,prior=self.invoke(self.reference(),['--reference','strict']);root=prior.parent
        full=(prior/'serial.log').read_bytes();partial=full.split(b'#### OS COMP TEST GROUP END')[0]
        image=root/'disk.img';kernel=root/'Image';init=root/'initramfs.gz'
        qemu=root/'qemu-system-riscv64';qemu.write_bytes(b'fake executable')
        info={'path':str(qemu),'invoked_path':str(qemu),'sha256':digest(qemu),'version':'QEMU emulator version test'}
        command=shlex.join(['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                           '-bios','default','-kernel',str(kernel),'-initrd',str(init),'-append','console=ttyS0 rdinit=/init',
                           '-drive',f'file={image},if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0'])
        import re
        bad_id=re.sub(rb'REF_BUILD_ID [0-9a-f]{64}',b'REF_BUILD_ID '+b'0'*64,partial)
        prefix=full.split(b'REF RUN')[0]
        cases=[('partial-exit',partial,False,7,'TIMEOUT'),('partial-zero',partial,False,0,'TIMEOUT'),
               ('bad-id',bad_id,False,7,'ENV_ERROR'),('bad-protocol',partial.replace(b'REF_PROTOCOL 1',b'REF_PROTOCOL 999'),True,-9,'ENV_ERROR'),
               ('explicit-fail',partial+b'REF_FAIL mount-ext4\n',False,7,'ENV_ERROR'),
               ('complete-prefix',full.split(b'REF_ALL_DONE')[0],False,7,'ENV_ERROR'),
               ('timeout-before-run',prefix,True,-9,'TIMEOUT'),('timeout-before-init',b'',True,-9,'TIMEOUT')]
        for index,(name,raw,timed_out,rc,status) in enumerate(cases):
            with self.subTest(case=name),patch.object(run_tests,'qemu_identity',return_value=info),patch.object(run_tests,'capture',return_value=(raw,timed_out,rc)),contextlib.redirect_stdout(io.StringIO()):
                out=root/str(index)
                code=run_tests.main(['--capture-cmd',command,'--out',str(out),'--libc','musl','--group','iperf','--reference','strict',
                                     '--inventory',str(root/'inventory.json'),'--reference-manifest',str(root/'build.manifest.json'),
                                     '--reference-image',str(image),'--reference-artifacts',str(kernel),str(init)])
                data=json.loads((out/'summary.json').read_text())
                self.assertEqual(code,1)
                self.assertEqual(data['groups'][0]['status'],status)
                self.assertFalse(data['meta']['execution_valid'])
                self.assertEqual(data['totals']['pass'],0)
                self.assertEqual(data['meta']['capture_exit_code'],rc)

    def test_malformed_judge_rows_return_judge_error(self):
        rows = [[{'name': 'BASIC_UDP', 'all': 0, 'pass': 0}],
                [{'name': 'BASIC_UDP', 'all': 1, 'pass': 2}],
                [{'name': 'BASIC_UDP', 'all': True, 'pass': 1}],
                [{'name': 'BASIC_UDP', 'all': 1, 'pass': 1, 'score': float('nan')}],
                [{'name': 'BASIC_UDP', 'all': 'oops', 'pass': 1}],
                [{'name': 'BASIC_UDP', 'all': 1, 'pass': 1}] * 2]
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / 'judge_iperf.py'
            for items in rows:
                with self.subTest(items=items):
                    fake.write_text('import json\nprint(' + repr(json.dumps(items)) + ')\n')
                    proc, data, _ = self.invoke(self.complete(), ['--team-judge-dir', tmp])
                    self.assertEqual(proc.returncode, 1)
                    self.assertEqual(data['groups'][0]['status'], 'JUDGE_ERROR')

    def test_strict_reference_requires_complete_protocol(self):
        proc, data, _ = self.invoke(self.reference().replace(b'REF_MODULES_OK\n',b''), ['--reference', 'strict'])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')

    def test_strict_required_parameters_are_checked_before_replay_or_capture(self):
        import contextlib,io,itertools,run_tests
        from unittest.mock import patch
        _,_,prior=self.invoke(self.reference(),['--reference','strict']);root=prior.parent
        options={'manifest':['--reference-manifest',str(root/'build.manifest.json')],
                 'artifacts':['--reference-artifacts',str(root/'Image'),str(root/'initramfs.gz')],
                 'image':['--reference-image',str(root/'disk.img')]}
        for count in [1,2,3]:
            for missing in itertools.combinations(options,count):
                for mode in ['capture','replay']:
                    with self.subTest(missing=missing,mode=mode):
                        source=['--capture-cmd','fake'] if mode=='capture' else ['--log',str(prior/'serial.log')]
                        args=source+['--capture-artifact',str(RUNNER),'--out',str(root/(mode+'-'.join(missing))),
                                     '--reference','strict','--libc','musl','--group','iperf']
                        for key,values in options.items():
                            if key not in missing:args+=values
                        with patch.object(run_tests,'capture',side_effect=AssertionError('capture executed')),patch.object(run_tests,'qemu_identity',side_effect=AssertionError('version probe executed')),patch.object(run_tests,'run_judge',side_effect=AssertionError('judge executed')),contextlib.redirect_stderr(io.StringIO()):
                            self.assertEqual(run_tests.main(args),2)
                        self.assertFalse((Path(args[args.index('--out')+1])/'summary.json').exists())

    def reference(self, rc=0):
        return (b'REF_PROTOCOL 1\nREF_MODULES_OK\nREF_MOUNT_OK\nREF_PREPARED musl\n'
                b'REF RUN musl iperf_testcode.sh\n' + self.complete() +
                f'REF RESULT musl iperf_testcode.sh {rc}\nREF_ALL_DONE\n'.encode())

    def test_valid_reference_protocol_and_test_exit_are_distinct(self):
        proc, data, _ = self.invoke(self.reference(), ['--reference', 'strict'])
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(data['meta']['environment']['status'], 'VALID')
        proc, data, _ = self.invoke(self.reference(3), ['--reference', 'strict'])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'FAIL')
        self.assertEqual(data['totals']['pass'], 0)
        self.assertEqual(data['meta']['environment']['status'], 'VALID')

    def test_reference_order_missing_and_duplicate_markers_are_errors(self):
        good = self.reference()
        bad = [good.replace(b'REF_MODULES_OK\n', b''),
               good.replace(b'REF_ALL_DONE', b'REF_ALL_DONE\nREF_ALL_DONE'),
               good.replace(b'REF_PREPARED musl\n', b'REF_PREPARED musl\nREF_PREPARED musl\n'),
               good.replace(b'REF RESULT musl', b'REF RESULT glibc'),
               good.replace(b'REF RESULT musl iperf_testcode.sh 0\n', b''),
               good.replace(b'REF_MOUNT_OK', b'REF_MOUNT_OK\nREF_FAIL configuration')]
        for raw in bad:
            with self.subTest(raw=raw):
                proc, data, _ = self.invoke(raw, ['--reference', 'strict'])
                self.assertEqual(proc.returncode, 1)
                self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')

    def test_legacy_reference_cannot_be_accepted_as_new_environment(self):
        raw = b'REF_MODULES_OK\nREF_MOUNT_OK\n' + self.complete() + b'REF_ALL_DONE\n'
        proc, data, _ = self.invoke(raw)
        self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')
        proc, data, _ = self.invoke(raw, ['--reference', 'legacy'])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'PASS')
        self.assertFalse(data['meta']['execution_valid'])

    def test_capture_timeout_terminates_descendants_even_after_success(self):
        import shlex
        import time
        import os
        with tempfile.TemporaryDirectory() as tmp:
            child_pid = Path(tmp)/'child.pid'
            source = ("import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)']);"
                      f"open({str(child_pid)!r},'w').write(str(p.pid));"
                      f"sys.stdout.buffer.write({self.complete()!r});sys.stdout.flush();time.sleep(30)")
            started = time.monotonic()
            proc, data, _ = self.invoke(capture=shlex.join([sys.executable, '-c', source]), extra=['--timeout', '1'])
            self.assertLess(time.monotonic()-started, 4)
            self.assertEqual(proc.returncode, 1)
            self.assertEqual(data['groups'][0]['status'], 'TIMEOUT')
            self.assertEqual(data['totals']['pass'], 0)
            stat = Path('/proc')/child_pid.read_text()/'stat'
            if stat.exists():
                self.assertEqual(stat.read_text().split()[2], 'Z')



    def test_crlf_reference_binds_manifest_and_preserves_serial_bytes(self):
        raw = self.reference().replace(b'\n', b'\r\n')
        proc, data, out = self.invoke(raw, ['--reference', 'strict'])
        self.assertEqual(proc.returncode, 0)
        self.assertTrue(data['meta']['reference_provenance']['inputs_pinned'])
        self.assertIn(b'REF_BUILD_ID ', (out/'serial.log').read_bytes())
        self.assertEqual((out/'serial.log').read_bytes().count(b'\r\n'), raw.count(b'\r\n')+1)

    def test_group_output_must_belong_to_active_reference_attempt(self):
        valid = self.reference()
        raw = valid.replace(self.complete(), b'')
        raw = raw.replace(b'REF RUN musl iperf_testcode.sh', self.complete()+b'REF RUN musl iperf_testcode.sh')
        proc, data, _ = self.invoke(raw, ['--reference', 'strict'])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')
        self.assertTrue(any('outside its reference attempt' in e for e in data['meta']['environment']['errors']))

    def test_reference_manifest_rejects_modified_artifact_or_unpinned_input(self):
        from reference_manifest import verify_manifest, build_identity
        from inventory import load_inventory
        proc, data, out = self.invoke(self.reference(), ['--reference', 'strict'])
        root = out.parent
        path = root/'build.manifest.json'
        manifest = json.loads(path.read_text())
        text = (out/'serial.log').read_bytes().decode()
        (root/'Image').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'artifact hash'):
            verify_manifest(path, [root/'Image', root/'initramfs.gz'], text, load_inventory(root/'inventory.json'), ['iperf'], 'musl', root/'inventory.json', image=root/'disk.img')
        (root/'Image').write_bytes(b'synthetic kernel')
        old = manifest['build_id']
        manifest['inputs_pinned'] = False
        manifest['build_id'] = build_identity(manifest)
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'not the frozen inputs'):
            verify_manifest(path, [root/'Image', root/'initramfs.gz'], text.replace(old, manifest['build_id']), load_inventory(root/'inventory.json'), ['iperf'], 'musl', root/'inventory.json', image=root/'disk.img')



    def test_shell_trace_does_not_create_fake_group_boundaries(self):
        raw = (b"+ echo '#### OS COMP TEST GROUP START iperf-musl ####'\n" + self.complete() +
               b"+ echo '#### OS COMP TEST GROUP END iperf-musl ####'\n")
        proc, data, _ = self.invoke(raw)
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(data['groups'][0]['status'], 'PASS')



    def test_complete_assertions_without_group_end_are_not_effective_passes(self):
        raw = self.complete().replace(b'#### OS COMP TEST GROUP END iperf-musl ####\n', b'')
        proc, data, _ = self.invoke(raw)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(data['groups'][0]['status'], 'TIMEOUT')
        self.assertEqual(data['totals']['pass'], 0)
        self.assertEqual(data['totals']['pass_rate'], 0)
        self.assertTrue(all(s['status'] == 'TIMEOUT' and s['pass'] == 0 for s in data['groups'][0]['subcases']))
        self.assertEqual(data['groups'][0]['subcases'][0]['observed']['status'], 'PASS')



    def test_capture_records_source_identity_before_external_execution(self):
        import contextlib
        import io
        import run_tests
        from unittest.mock import patch
        original_digest = run_tests.digest
        started = False
        def capture(command, timeout):
            nonlocal started
            started = True
            return self.complete(), False, 0
        def digest(path):
            if Path(path).resolve() == RUNNER.resolve():
                return 'b'*64 if started else 'a'*64
            return original_digest(path)
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(run_tests, 'capture', side_effect=capture), patch.object(run_tests, 'digest', side_effect=digest), contextlib.redirect_stdout(io.StringIO()):
                code = run_tests.main(['--capture-cmd', 'fake', '--capture-artifact', str(RUNNER), '--out', tmp, '--libc', 'musl',
                                       '--group', 'iperf', '--inventory', str(INVENTORY),
                                       '--team-judge-dir', str(TEAM_DIR)])
            data = json.loads((Path(tmp)/'summary.json').read_text())
            self.assertEqual(code, 1)
            self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')
            self.assertEqual(data['meta']['source_sha256']['scripts/run_tests.py'], 'a'*64)


    def test_score_does_not_create_partial_assertions(self):
        import run_tests
        from unittest.mock import patch
        cfg = {'kind': 'team', 'judge_file': 'judge_iperf.py', 'subcases': ['a']}
        for passed, score, status in [(0, 10, 'FAIL'), (1, 0, 'PARTIAL'), (2, 0, 'PASS')]:
            with self.subTest(passed=passed), patch.object(run_tests, 'run_judge', return_value=run_tests.JudgeResult(
                    [{'name': 'a', 'all': 2, 'pass': passed, 'score': score}], 'ok')):
                result = run_tests.judge_group('iperf', run_tests.Section('iperf', 'musl'), cfg, Path('.'), Path('.'), 1)
                self.assertEqual(result['status'], status)
                self.assertEqual(result['subcases'][0]['status'], status)

    def test_unknown_team_markers_are_reported(self):
        import run_tests
        from inventory import load_inventory
        for group in ['iperf', 'netperf', 'cyclictest']:
            text = f'====== {group} NEW_CASE end: success ======\n'
            cfg = load_inventory()['groups'][group]
            result = run_tests.judge_group(group, run_tests.Section(group, 'musl', text.splitlines()), cfg, Path('.'), TEAM_DIR, 5)
            self.assertEqual(result['unexpected_in_log'], ['NEW_CASE'])

    def test_inventory_changes_during_capture_or_judging_invalidate_result(self):
        import contextlib
        import io
        import run_tests
        from inventory import digest
        from unittest.mock import patch
        for stage in ['capture', 'judge']:
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as tmp:
                inventory = Path(tmp)/'inventory.json'
                inventory.write_bytes(INVENTORY.read_bytes())
                original_hash = digest(inventory)
                original_judge = run_tests.judge_group
                def mutate():
                    inventory.write_bytes(inventory.read_bytes()+b' ')
                def capture(command, timeout):
                    if stage == 'capture': mutate()
                    return self.complete(), False, 0
                def judge(*args, **kwargs):
                    result = original_judge(*args, **kwargs)
                    if stage == 'judge': mutate()
                    return result
                with patch.object(run_tests, 'capture', side_effect=capture), patch.object(run_tests, 'judge_group', side_effect=judge), contextlib.redirect_stdout(io.StringIO()):
                    code = run_tests.main(['--capture-cmd', 'fake', '--capture-artifact', str(RUNNER), '--out', tmp, '--libc', 'musl',
                                           '--group', 'iperf', '--inventory', str(inventory), '--team-judge-dir', str(TEAM_DIR)])
                data = json.loads((Path(tmp)/'summary.json').read_text())
                self.assertEqual(code, 1)
                self.assertEqual(data['meta']['inventory_sha256'], original_hash)
                self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')
                self.assertEqual(data['totals']['pass'], 0)

    def test_judge_or_reference_lock_change_is_detected_before_judging(self):
        import contextlib
        import io
        import run_tests
        from inventory import digest
        from unittest.mock import patch
        for kind in ['judge', 'lock']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                team = root/'team'
                team.mkdir()
                judge = team/'judge_iperf.py'
                judge.write_bytes((TEAM_DIR/'judge_iperf.py').read_bytes())
                lock = root/'reference.lock.json'
                lock.write_text('{}')
                target = judge if kind == 'judge' else lock
                before = digest(target)
                def capture(command, timeout):
                    target.write_bytes(target.read_bytes()+b' ')
                    return self.complete(), False, 0
                with patch.object(run_tests, 'REFERENCE_LOCK', lock), patch.object(run_tests, 'capture', side_effect=capture), patch.object(run_tests, 'run_judge', side_effect=AssertionError('changed judge executed')), contextlib.redirect_stdout(io.StringIO()):
                    code = run_tests.main(['--capture-cmd', 'fake', '--capture-artifact', str(RUNNER), '--out', str(root/'out'), '--libc', 'musl',
                                           '--group', 'iperf', '--inventory', str(INVENTORY), '--team-judge-dir', str(team)])
                data = json.loads((root/'out/summary.json').read_text())
                self.assertEqual(code, 1)
                self.assertEqual(data['groups'][0]['status'], 'ENV_ERROR')
                self.assertIn(str(target), data['meta']['changed_inputs'])
                if kind == 'judge': self.assertEqual(data['meta']['judge_sha256'][str(judge)], before)

    def test_reused_output_removes_stale_sections(self):
        import run_tests, contextlib, io
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); log = root/'input.log'; out = root/'out'
            base = ['--log', str(log), '--out', str(out), '--libc', 'musl', '--group', 'iperf',
                    '--inventory', str(INVENTORY), '--team-judge-dir', str(TEAM_DIR)]
            log.write_bytes(self.complete())
            with contextlib.redirect_stdout(io.StringIO()): self.assertEqual(run_tests.main(base), 0)
            self.assertTrue((out/'sections/iperf-musl.log').exists())
            log.write_bytes(b'')
            with contextlib.redirect_stdout(io.StringIO()): self.assertEqual(run_tests.main(base), 1)
            self.assertEqual(list((out/'sections').iterdir()), [])

    def test_alternative_team_case_list_is_rejected_before_capture(self):
        import run_tests, contextlib, io
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'inventory.json'; data=json.loads(INVENTORY.read_text())
            data['groups']['iperf']['subcases'][0]='NEW_CASE';path.write_text(json.dumps(data))
            with patch.object(run_tests, 'capture', side_effect=AssertionError('capture executed')), contextlib.redirect_stderr(io.StringIO()):
                code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(RUNNER),'--out',str(Path(tmp)/'out'),
                                    '--libc','musl','--group','iperf','--inventory',str(path)])
            self.assertEqual(code,2)

    def test_plain_capture_artifact_is_hashed_and_changes_invalidate(self):
        import run_tests, contextlib, io
        from inventory import digest
        from unittest.mock import patch
        for mutate in [False,True]:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp); artifact=root/'kernel';artifact.write_bytes(b'kernel v1');before=digest(artifact)
                def capture(cmd, timeout):
                    if mutate: artifact.write_bytes(b'kernel v2')
                    return self.complete(),False,0
                with patch.object(run_tests,'capture',side_effect=capture), contextlib.redirect_stdout(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(artifact),'--out',str(root/'out'),
                                        '--libc','musl','--group','iperf','--team-judge-dir',str(TEAM_DIR)])
                result=json.loads((root/'out/summary.json').read_text())
                self.assertEqual(result['meta']['artifact_sha256'][str(artifact)],before)
                self.assertEqual(code,1 if mutate else 0)
                self.assertEqual(result['groups'][0]['status'],'ENV_ERROR' if mutate else 'PASS')

    def test_managed_output_cannot_replace_artifact_or_follow_sections_symlink(self):
        import run_tests,contextlib,io
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); out=root/'out';out.mkdir()
            artifact=out/'serial.log';artifact.write_bytes(b'kernel')
            args=['--capture-cmd','fake','--capture-artifact',str(artifact),'--out',str(out),'--libc','musl','--group','iperf']
            with patch.object(run_tests,'capture',side_effect=AssertionError('capture executed')),contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(run_tests.main(args),2)
            self.assertEqual(artifact.read_bytes(),b'kernel')
            target=root/'external';target.mkdir();(target/'keep').write_text('keep')
            (out/'sections').symlink_to(target,target_is_directory=True)
            with contextlib.redirect_stderr(io.StringIO()):self.assertEqual(run_tests.main(args),2)
            self.assertEqual((target/'keep').read_text(),'keep')

    def test_capture_without_declared_artifacts_fails_before_execution(self):
        import run_tests,contextlib,io
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp,patch.object(run_tests,'capture',side_effect=AssertionError('capture executed')),contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(run_tests.main(['--capture-cmd','fake','--out',tmp,'--libc','musl']),2)

    def test_log_cannot_alias_managed_summaries(self):
        import run_tests,contextlib,io,os
        for name in ['summary.json','summary.md']:
            for hardlink in [False,True]:
                with self.subTest(name=name,hardlink=hardlink),tempfile.TemporaryDirectory() as tmp:
                    root=Path(tmp);out=root/'out';out.mkdir();source=out/name;source.write_bytes(self.complete())
                    log=root/'input.log' if hardlink else source
                    if hardlink: os.link(source,log)
                    with contextlib.redirect_stderr(io.StringIO()),contextlib.redirect_stdout(io.StringIO()):
                        code=run_tests.main(['--log',str(log),'--out',str(out),'--libc','musl','--group','iperf','--team-judge-dir',str(TEAM_DIR)])
                    self.assertEqual(code,2)
                    self.assertEqual(source.read_bytes(),self.complete())
                    self.assertEqual(log.read_bytes(),self.complete())

    def test_strict_capture_checks_declared_qemu_paths_and_no_extra_drives(self):
        from reference_manifest import strict_qemu_command
        import shlex
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);kernel=root/'Image';init=root/'init.gz';image=root/'disk.img'
            args=['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                  '-bios','default','-kernel',str(kernel),'-initrd',str(init),'-append','console=ttyS0 rdinit=/init',
                  '-drive',f'file={image},if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0']
            self.assertEqual(strict_qemu_command(shlex.join(args),[kernel,init],image),args)
            for old,new in [(str(kernel),str(root/'wrong-kernel')), (str(init),str(root/'wrong-init')),
                            (f'file={image},if=none,format=raw,id=x0',f'file={root}/wrong.img,if=none,format=raw,id=x0')]:
                bad=[new if arg==old else arg for arg in args]
                with self.subTest(new=new),self.assertRaises(ValueError): strict_qemu_command(shlex.join(bad),[kernel,init],image)
            for extra in [['-drive','file=other.img'],[';','echo','ignored']]:
                with self.assertRaises(ValueError):strict_qemu_command(shlex.join(args+extra),[kernel,init],image)

    def test_reference_disk_replacement_or_missing_disk_is_not_accepted(self):
        from reference_manifest import verify_manifest
        from inventory import load_inventory
        proc, data, out = self.invoke(self.reference(), ['--reference','strict'])
        self.assertEqual(proc.returncode,0)
        root=out.parent;image=root/'disk.img';inventory=root/'inventory.json'
        args=[root/'build.manifest.json',[root/'Image',root/'initramfs.gz'],(out/'serial.log').read_text(),load_inventory(inventory),['iperf'],'musl',inventory]
        with self.assertRaisesRegex(ValueError,'disk'):verify_manifest(*args)
        image.write_bytes(b'other disk')
        with self.assertRaisesRegex(ValueError,'disk'):verify_manifest(*args,image=image)

    def test_inplace_serial_replay_preserves_exact_bytes(self):
        import run_tests,contextlib,io
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp);raw=out/'serial.log';raw.write_bytes(self.complete())
            with contextlib.redirect_stdout(io.StringIO()):
                code=run_tests.main(['--log',str(raw),'--out',str(out),'--libc','musl','--group','iperf','--team-judge-dir',str(TEAM_DIR)])
            self.assertEqual(code,0)
            self.assertEqual(raw.read_bytes(),self.complete())

    def test_strict_disk_changed_before_or_during_capture_never_passes(self):
        import run_tests,contextlib,io,shlex
        from unittest.mock import patch
        for stage in ['before','during','qemu']:
            proc,data,out=self.invoke(self.reference(),['--reference','strict']);root=out.parent
            image=root/'disk.img';kernel=root/'Image';init=root/'initramfs.gz'
            command=shlex.join(['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                               '-bios','default','-kernel',str(kernel),'-initrd',str(init),'-append','console=ttyS0 rdinit=/init',
                               '-drive',f'file={image},if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0'])
            qemu=root/'qemu-system-riscv64';qemu.write_bytes(b'fake executable')
            from inventory import digest
            info={'path':str(qemu),'invoked_path':str(qemu),'sha256':digest(qemu),'version':'QEMU emulator version test'}
            if stage=='before':image.write_bytes(b'replaced disk')
            def capture(cmd,timeout):
                self.assertIsInstance(cmd,list)
                (qemu if stage=='qemu' else image).write_bytes(b'mutated while running')
                return (out/'serial.log').read_bytes(),False,0
            dest=root/'capture-out'
            with self.subTest(stage=stage),patch.object(run_tests,'qemu_identity',return_value=info),patch.object(run_tests,'capture',side_effect=capture) as mocked,contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
                code=run_tests.main(['--capture-cmd',command,'--out',str(dest),'--libc','musl','--group','iperf',
                                     '--reference','strict','--reference-image',str(image),'--inventory',str(root/'inventory.json'),
                                     '--reference-manifest',str(root/'build.manifest.json'),'--reference-artifacts',str(kernel),str(init),
                                     '--team-judge-dir',str(TEAM_DIR)])
            self.assertEqual(code,2 if stage=='before' else 1)
            if stage=='before':mocked.assert_not_called()
            else:
                result=json.loads((dest/'summary.json').read_text())
                self.assertEqual(result['groups'][0]['status'],'ENV_ERROR')
                self.assertEqual(result['totals']['pass'],0)

    def test_output_symlink_created_during_capture_cannot_overwrite_input(self):
        import run_tests,contextlib,io
        from unittest.mock import patch
        for name in ['serial.log','summary.json','summary.md','sections']:
            with self.subTest(name=name),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);out=root/'out';artifact=root/'kernel';artifact.write_bytes(b'original kernel')
                external=root/'external';external.mkdir();(external/'keep').write_text('keep')
                def capture(cmd,timeout):
                    out.mkdir()
                    (out/name).symlink_to(external if name=='sections' else artifact,target_is_directory=name=='sections')
                    return self.complete(),False,0
                with patch.object(run_tests,'capture',side_effect=capture),contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(artifact),'--out',str(out),
                                        '--libc','musl','--group','iperf','--team-judge-dir',str(TEAM_DIR)])
                self.assertEqual(code,2)
                self.assertEqual(artifact.read_bytes(),b'original kernel')
                self.assertEqual((external/'keep').read_text(),'keep')

    def test_strict_command_executes_canonical_artifact_paths(self):
        from reference_manifest import strict_qemu_command
        import shlex
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);kernel=root/'Image';init=root/'init.gz';image=root/'disk.img'
            for p in [kernel,init,image]:p.write_bytes(b'artifact')
            aliases=[]
            for p in [kernel,init,image]:
                alias=root/('alias-'+p.name);alias.symlink_to(p);aliases.append(alias)
            command=shlex.join(['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                               '-bios','default','-kernel',str(aliases[0]),'-initrd',str(aliases[1]),'-append','console=ttyS0 rdinit=/init',
                               '-drive',f'file={aliases[2]},if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0'])
            argv=strict_qemu_command(command,aliases[:2],aliases[2])
            self.assertEqual(argv[13],str(kernel))
            self.assertEqual(argv[15],str(init))
            self.assertEqual(argv[19],f'file={image},if=none,format=raw,id=x0')

    def test_log_replay_has_source_and_replay_time_not_capture_time(self):
        proc,data,out=self.invoke(self.complete())
        self.assertEqual(proc.returncode,0)
        self.assertEqual(data['meta']['mode'],'replay')
        self.assertNotIn('captured_at',data['meta'])
        self.assertIn('replayed_at',data['meta'])
        self.assertEqual(data['meta']['replayed_from']['sha256'],data['meta']['log_sha256'])

    def test_script_failure_does_not_hide_judge_error(self):
        for contents in [None,'raise SystemExit(3)','print("not json")']:
            with self.subTest(contents=contents),tempfile.TemporaryDirectory() as tmp:
                if contents is not None:(Path(tmp)/'judge_iperf.py').write_text(contents)
                proc,data,out=self.invoke(self.reference(3),['--reference','strict','--team-judge-dir',tmp])
                self.assertEqual(proc.returncode,1)
                self.assertEqual(data['groups'][0]['status'],'JUDGE_ERROR')
                self.assertEqual(data['totals']['groups']['JUDGE_ERROR'],1)
                self.assertEqual(data['meta']['environment']['script_exits']['iperf-musl'],3)

    def test_qemu_identity_resolves_path_and_rejects_version_probe_changes(self):
        from reference_manifest import qemu_identity
        from unittest.mock import patch
        from inventory import digest
        with tempfile.TemporaryDirectory() as tmp:
            binary=Path(tmp)/'real-qemu';alias=Path(tmp)/'qemu-system-riscv64'
            binary.write_text('#!/bin/sh\necho "QEMU emulator version test"\n');binary.chmod(0o755);alias.symlink_to(binary)
            with patch.dict(os.environ,{'PATH':tmp}):info=qemu_identity('qemu-system-riscv64')
            self.assertEqual(info['path'],str(binary))
            self.assertEqual(info['sha256'],digest(binary))
            self.assertEqual(info['version'],'QEMU emulator version test')
            binary.write_text('#!/bin/sh\necho "# changed" >> "$0"\necho "QEMU emulator version test"\n')
            with self.assertRaises(ValueError):qemu_identity(str(alias))

    def test_old_log_cannot_be_paired_with_different_artifacts_and_updated_manifest(self):
        from reference_manifest import verify_manifest, build_identity
        from inventory import load_inventory, digest
        for artifact in ['Image','initramfs.gz']:
            for rewrite_content in [False,True]:
                with self.subTest(artifact=artifact,rewrite_content=rewrite_content):
                    proc,data,out=self.invoke(self.reference(),['--reference','strict'])
                    self.assertEqual(proc.returncode,0)
                    root=out.parent;path=root/'build.manifest.json';manifest=json.loads(path.read_text())
                    changed=root/artifact;changed.write_bytes(changed.read_bytes()+b'changed artifact')
                    key='kernel_sha256' if artifact=='Image' else 'initramfs_sha256'
                    manifest['outputs'][key]=digest(changed)
                    if rewrite_content and 'content' in manifest:
                        manifest['content'][key if artifact=='Image' else 'initramfs_payload_sha256']=digest(changed)
                        manifest['build_id']=build_identity(manifest)
                    path.write_text(json.dumps(manifest))
                    with self.assertRaises(ValueError):
                        verify_manifest(path,[root/'Image',root/'initramfs.gz'],(out/'serial.log').read_text(),
                                        load_inventory(root/'inventory.json'),['iperf'],'musl',root/'inventory.json',image=root/'disk.img')

    def test_truncated_section_retains_timeout_when_judge_fails(self):
        import run_tests
        from unittest.mock import patch
        cfg={'kind':'team','judge_file':'judge_iperf.py','subcases':['one']}
        cases=[run_tests.JudgeResult([], 'error', 'missing/crashed judge'),
               run_tests.JudgeResult([{'name':'one','all':1,'pass':2}], 'ok'),
               run_tests.JudgeResult([{'name':'one','all':1,'pass':1}]*2, 'ok')]
        for judged in cases:
            with self.subTest(judged=judged), patch.object(run_tests,'run_judge',return_value=judged):
                result=run_tests.judge_group('iperf',run_tests.Section('iperf','musl',[],True),cfg,Path('.'),Path('.'),1)
                self.assertEqual(result['status'],'TIMEOUT')
                self.assertTrue(result['judge_error'])
                self.assertTrue(all(sub['status']=='TIMEOUT' and sub['pass']==0 for sub in result['subcases']))
        for contents in [None,'raise SystemExit(3)','print("not json")']:
            with self.subTest(contents=contents),tempfile.TemporaryDirectory() as tmp:
                if contents is not None:(Path(tmp)/'judge_iperf.py').write_text(contents)
                raw=self.complete().replace(b'#### OS COMP TEST GROUP END iperf-musl ####\n',b'')
                proc,data,_=self.invoke(raw,['--team-judge-dir',tmp])
                self.assertEqual(proc.returncode,1)
                self.assertEqual(data['groups'][0]['status'],'TIMEOUT')
                self.assertTrue(data['groups'][0]['judge_error'])
                self.assertFalse(data['meta']['execution_valid'])

    def test_schema_one_manifest_cannot_claim_strict_artifact_binding(self):
        from reference_manifest import verify_manifest,build_identity
        from inventory import load_inventory
        _,_,out=self.invoke(self.reference(),['--reference','strict'])
        root=out.parent;path=root/'build.manifest.json';manifest=json.loads(path.read_text())
        manifest['schema_version']=1;manifest.pop('content')
        manifest['build_id']=build_identity(manifest);path.write_text(json.dumps(manifest))
        with self.assertRaises(ValueError):
            verify_manifest(path,[root/'Image',root/'initramfs.gz'],(out/'serial.log').read_text(),
                            load_inventory(root/'inventory.json'),['iperf'],'musl',root/'inventory.json',image=root/'disk.img')

    def test_invalid_manifest_structure_is_argument_error_before_execution(self):
        import contextlib,copy,io,run_tests,shlex
        from unittest.mock import patch
        _,_,out=self.invoke(self.reference(),['--reference','strict'])
        root=out.parent;path=root/'build.manifest.json';valid=json.loads(path.read_text())
        malformed=[[],None,False,'manifest',{'inputs':[]}]
        for field in ['inputs','selection','outputs','content']:
            for value in [[],None,'bad',1]:
                bad=copy.deepcopy(valid);bad[field]=value;malformed.append(bad)
        for field,value in [('libcs',[{}]),('scripts',[[]]),('libcs','musl'),('release',[])]:
            bad=copy.deepcopy(valid);bad['selection'][field]=value;malformed.append(bad)
        for mode in ['capture','replay']:
            for index,bad in enumerate(malformed):
                with self.subTest(mode=mode,bad=bad):
                    path.write_text(json.dumps(bad));dest=root/f'{mode}-{index}'
                    command=shlex.join(['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                                       '-bios','default','-kernel',str(root/'Image'),'-initrd',str(root/'initramfs.gz'),'-append','console=ttyS0 rdinit=/init',
                                       '-drive',f'file={root}/disk.img,if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0'])
                    source=['--capture-cmd',command] if mode=='capture' else ['--log',str(out/'serial.log')]
                    with patch.object(run_tests,'qemu_identity',side_effect=AssertionError('version probe executed')),patch.object(run_tests,'capture',side_effect=AssertionError('capture executed')),patch.object(run_tests,'run_judge',side_effect=AssertionError('judge executed')),contextlib.redirect_stderr(io.StringIO()):
                        code=run_tests.main(source+['--out',str(dest),'--libc','musl','--group','iperf','--reference','strict',
                                                   '--inventory',str(root/'inventory.json'),'--reference-manifest',str(path),
                                                   '--reference-image',str(root/'disk.img'),'--reference-artifacts',str(root/'Image'),str(root/'initramfs.gz')])
                    self.assertEqual(code,2)
                    self.assertFalse(dest.exists())

    def test_schema_two_requires_each_implementation_and_busybox_identity(self):
        from reference_manifest import load_manifest,build_identity
        for field in ['builder_sha256','inventory_code_sha256','manifest_code_sha256','busybox_sha256']:
            with self.subTest(field=field):
                _,_,out=self.invoke(self.reference(),['--reference','strict'])
                path=out.parent/'build.manifest.json';manifest=json.loads(path.read_text())
                del manifest[field];manifest['build_id']=build_identity(manifest)
                path.write_text(json.dumps(manifest))
                with self.assertRaises(ValueError):load_manifest(path)

    def test_strict_artifact_errors_stop_before_any_qemu_or_judge_execution(self):
        import contextlib,gzip,io,run_tests,shlex
        from reference_manifest import build_identity
        from inventory import digest
        from unittest.mock import patch
        for error in ['kernel','initramfs','gzip','kernel-content','payload-content','busybox-content','unpinned','selection']:
            with self.subTest(error=error):
                _,_,old_out=self.invoke(self.reference(),['--reference','strict'])
                root=old_out.parent;path=root/'build.manifest.json';kernel=root/'Image';init=root/'initramfs.gz';image=root/'disk.img'
                manifest=json.loads(path.read_text());old_id=manifest['build_id']
                if error=='kernel':kernel.write_bytes(b'changed kernel')
                elif error=='initramfs':init.write_bytes(b'changed initramfs')
                elif error=='gzip':
                    init.write_bytes(b'not gzip');manifest['outputs']['initramfs_sha256']=digest(init)
                else:
                    if error=='kernel-content':manifest['content']['kernel_sha256']='0'*64
                    elif error=='payload-content':manifest['content']['initramfs_payload_sha256']='0'*64
                    elif error=='busybox-content':manifest['busybox_sha256']='0'*64
                    elif error=='unpinned':manifest['inputs_pinned']=False
                    elif error=='selection':manifest['selection']['scripts']=['lua_testcode.sh']
                    manifest['build_id']=build_identity(manifest)
                    init.write_bytes(gzip.compress(gzip.decompress(init.read_bytes()).replace(old_id.encode(),manifest['build_id'].encode()),mtime=0))
                    manifest['outputs']['initramfs_sha256']=digest(init)
                path.write_text(json.dumps(manifest))
                command=shlex.join(['qemu-system-riscv64','-machine','virt','-m','1G','-smp','1','-nographic','-no-reboot','-snapshot',
                                   '-bios','default','-kernel',str(kernel),'-initrd',str(init),'-append','console=ttyS0 rdinit=/init',
                                   '-drive',f'file={image},if=none,format=raw,id=x0','-device','virtio-blk-device,drive=x0'])
                for mode in ['capture','replay']:
                    dest=root/mode
                    source=['--capture-cmd',command] if mode=='capture' else ['--log',str(old_out/'serial.log')]
                    with patch.object(run_tests,'qemu_identity',side_effect=AssertionError('QEMU version probe ran')),patch.object(run_tests,'capture',side_effect=AssertionError('capture ran')),patch.object(run_tests,'run_judge',side_effect=AssertionError('judge ran')),contextlib.redirect_stderr(io.StringIO()):
                        code=run_tests.main(source+['--out',str(dest),'--libc','musl','--group','iperf','--reference','strict',
                                                   '--inventory',str(root/'inventory.json'),'--reference-manifest',str(path),
                                                   '--reference-image',str(image),'--reference-artifacts',str(kernel),str(init)])
                    self.assertEqual(code,2)
                    self.assertFalse(dest.exists())

    def test_large_artifact_is_hashed_twice_regardless_of_groups_and_aliases(self):
        import contextlib,io,run_tests
        from unittest.mock import patch
        for groups in [['iperf'],None]:
            with self.subTest(groups=groups),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);image=root/'disk';image.write_bytes(b'disk'*16384)
                symbolic=root/'symbolic';symbolic.symlink_to(image)
                hard=root/'hard';os.link(image,hard)
                paths=[os.path.relpath(image),str(image),str(symbolic),str(hard)]
                reads=[];original=run_tests.digest
                def digest(path):
                    if Path(path).is_file() and Path(path).samefile(image):reads.append(str(path))
                    return original(path)
                args=['--capture-cmd','fake','--out',str(root/'out'),'--libc','musl']
                for path in paths:args+=['--capture-artifact',path]
                for group in groups or []:args+=['--group',group]
                with patch.object(run_tests,'digest',side_effect=digest),patch.object(run_tests,'capture',return_value=(self.complete(),False,0)),contextlib.redirect_stdout(io.StringIO()):
                    run_tests.main(args)
                self.assertEqual(len(reads),2,reads)
                data=json.loads((root/'out/summary.json').read_text())
                self.assertEqual(len(data['meta']['artifact_sha256']),4)

    def test_loaded_source_identity_is_frozen_before_artifact_hashing(self):
        import contextlib,io,run_tests
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            artifact=Path(tmp)/'disk';artifact.write_bytes(b'disk')
            hashing_started=False;original=run_tests.digest
            def digest(path):
                nonlocal hashing_started
                if Path(path).resolve()==artifact:
                    hashing_started=True
                if Path(path).resolve()==RUNNER:
                    return 'b'*64 if hashing_started else 'a'*64
                return original(path)
            with patch.object(run_tests,'digest',side_effect=digest),patch.object(run_tests,'capture',side_effect=AssertionError('capture ran with changed source')),contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(run_tests.main(['--capture-cmd','fake','--capture-artifact',str(artifact),'--out',str(Path(tmp)/'out'),'--libc','musl','--group','iperf']),2)

    def test_artifact_alias_retarget_and_same_hash_rewrite_during_judging_are_detected(self):
        import contextlib,io,run_tests
        from unittest.mock import patch
        for change in ['retarget','rewrite']:
            with self.subTest(change=change),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);image=root/'disk';other=root/'other';image.write_bytes(b'disk');other.write_bytes(b'disk')
                alias=root/'alias';alias.symlink_to(image);initial=image.stat();original=run_tests.judge_group
                def judge(*args,**kwargs):
                    result=original(*args,**kwargs)
                    if change=='retarget':alias.unlink();alias.symlink_to(other)
                    else:
                        image.write_bytes(b'XXXX');image.write_bytes(b'disk')
                        os.utime(image,ns=(initial.st_atime_ns,initial.st_mtime_ns))
                    return result
                with patch.object(run_tests,'capture',return_value=(self.complete(),False,0)),patch.object(run_tests,'judge_group',side_effect=judge),contextlib.redirect_stdout(io.StringIO()):
                    code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(alias),'--out',str(root/'out'),'--libc','musl','--group','iperf'])
                data=json.loads((root/'out/summary.json').read_text())
                self.assertEqual(code,1)
                self.assertEqual(data['groups'][0]['status'],'ENV_ERROR')
                self.assertFalse(data['meta']['execution_valid'])

    def test_unknown_testcase_lines_are_recorded_for_lua_and_busybox(self):
        from inventory import load_inventory
        for group,name in [('lua','lua added.lua'),('busybox','echo new command')]:
            cases=load_inventory()['groups'][group]['subcases']
            for status in ['success','fail','invalid']:
                with self.subTest(group=group,status=status):
                    text=f'#### OS COMP TEST GROUP START {group}-musl ####\n'
                    text+=''.join(f'testcase {case} success\n' for case in cases)
                    text+=f'testcase {name} {status}\n'
                    text+=f'+ echo testcase {name} trace-only success\ndiagnostic testcase {name} embedded success\n'
                    text+=f'#### OS COMP TEST GROUP END {group}-musl ####\n'
                    code,data=run(text,[group])
                    self.assertEqual(code,0)
                    self.assertEqual(data['groups'][0]['unexpected_in_log'],[name])
                    self.assertEqual(data['totals']['subcases'],len(cases))
                    self.assertEqual(data['totals']['pass'],len(cases))

    def test_managed_output_hardlinks_are_rejected_before_capture_and_before_write(self):
        import run_tests,contextlib,io
        from itertools import combinations
        from unittest.mock import patch
        for left,right in combinations(['serial.log','summary.json','summary.md'],2):
            for stage in ['before','capture']:
                with self.subTest(left=left,right=right,stage=stage),tempfile.TemporaryDirectory() as tmp:
                    root=Path(tmp);out=root/'out';out.mkdir()
                    def link():
                        (out/left).write_bytes(b'preserve');os.link(out/left,out/right)
                    if stage=='before':link()
                    def capture(cmd,timeout):
                        if stage=='before':self.fail('capture executed with colliding outputs')
                        link()
                        return self.complete(),False,0
                    with patch.object(run_tests,'capture',side_effect=capture),contextlib.redirect_stderr(io.StringIO()),contextlib.redirect_stdout(io.StringIO()):
                        code=run_tests.main(['--capture-cmd','fake','--capture-artifact',str(RUNNER),'--out',str(out),
                                            '--libc','musl','--group','iperf','--team-judge-dir',str(TEAM_DIR)])
                    self.assertEqual(code,2)
                    self.assertEqual((out/left).read_bytes(),b'preserve')
                    self.assertEqual((out/right).read_bytes(),b'preserve')

if __name__ == '__main__':
    unittest.main()
