"""静态目录与脚本交叉审计；合成目录不作为来宾执行证据。"""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test001_runtime_audit as runtime


class RuntimeAuditTests(unittest.TestCase):
    def test_missing_interpreters_and_no_shebang_are_explicit(self):
        result = runtime.audit()
        self.assertFalse(result['runtime_ready'])
        self.assertEqual(len(result['scripts']), 8)
        for row in result['scripts']:
            if row['source_file'].endswith('/lua_testcode.sh'):
                self.assertIsNone(row['interpreter'])
                self.assertEqual(row['interpreter_status'], 'NO_SHEBANG')
            else:
                self.assertEqual(row['interpreter_status'], 'ABSENT_FROM_BASE_IMAGE')
        self.assertEqual({r['interpreter'] for r in result['scripts']},
                         {None, '/busybox', '/bin/busybox', '/bin/bash'})

    def test_unknown_directory_is_not_absence(self):
        layout = {'/': {'musl': {'kind': 'directory'}},
                  '/musl': {'lib': {'kind': 'directory'}, 'lua': {'kind': 'regular'}}}
        self.assertEqual(runtime.probe_path('/musl/lib/libc.so', layout)['status'], 'NOT_AUDITED')
        self.assertEqual(runtime.probe_path('/bin/busybox', layout)['status'], 'ABSENT_FROM_BASE_IMAGE')
        self.assertEqual(runtime.probe_path('/musl/lua', layout)['status'], 'PRESENT_IN_BASE_IMAGE')
        self.assertEqual(runtime.probe_path('/musl/lua/child', layout)['status'], 'ABSENT_FROM_BASE_IMAGE')

    def test_noncanonical_paths_rejected(self):
        for path in ('musl/lua', '/musl/../busybox', '/musl//lua', '/musl/./lua', '/'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                runtime.probe_path(path, {})

    def test_programs_exist_and_write_targets_start_absent(self):
        result = runtime.audit()
        for group in result['workspaces'].values():
            self.assertEqual(set(group['programs']), {'busybox', 'lua', 'test.sh'})
            self.assertTrue(all(v['status'] == 'PRESENT_IN_BASE_IMAGE' for v in group['programs'].values()))
            self.assertEqual(set(group['write_targets']), {'test.txt', 'test_dir', 'test', 'busybox_cmd.bak'})
            self.assertTrue(all(v['status'] == 'ABSENT_FROM_BASE_IMAGE' for v in group['write_targets'].values()))

    def test_mixed_image_locks_rejected(self):
        loader = runtime.common.load_inputs
        def load(*args):
            data, lock = loader(*args)
            if args[0] == runtime.ltp.INPUTS:
                lock['image']['uncompressed_sha256'] = '0' * 64
            return data, lock
        with patch.object(runtime.common, 'load_inputs', side_effect=load):
            with self.assertRaisesRegex(ValueError, 'different images'):
                runtime.audit()

    def test_report_regenerates(self):
        report = runtime.audit()
        self.assertEqual(report, json.loads((runtime.common.ROOT / 'tests/test001/runtime-audit-report.json').read_text()))

    def test_root_directory_verification_and_path_allowlist(self):
        raw, listing = b'synthetic', b'root listing'
        common = runtime.common
        lock = {'image': {'uncompressed_size': len(raw), 'uncompressed_sha256': common.digest(raw)},
                'files': {'root': {'source': 'image', 'operation': 'ls -p', 'path': '/',
                                   'size': len(listing), 'sha256': common.digest(listing)}},
                'executables': {}}
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            image.write_bytes(raw)
            with patch.object(common.shutil, 'which', return_value='/debugfs'), \
                 patch.object(common, 'run_debugfs', side_effect=[
                     subprocess.CompletedProcess([], 0, listing, b''),
                     subprocess.CompletedProcess([], 0, b'', b'version')]) as run:
                self.assertEqual(common.verify_image(image, lock)['members'], 1)
                self.assertEqual(run.call_args_list[0].args[0][1], 'ls -p /')
            for operation, path in (('cat', '/'), ('ls -p', '/;'), ('ls -p', '/../')):
                lock['files']['root'].update(operation=operation, path=path)
                with patch.object(common.shutil, 'which', return_value='/debugfs'), \
                     patch.object(common, 'run_debugfs') as run:
                    with self.assertRaises(ValueError):
                        common.verify_image(image, lock)
                    run.assert_not_called()

    def test_cli_failure_emits_no_json(self):
        for error, code in ((ValueError('input'), 2), (subprocess.SubprocessError('tool'), 1)):
            out, err = io.StringIO(), io.StringIO()
            with patch('sys.argv', ['audit']), patch.object(runtime, 'audit', side_effect=error), \
                 contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                self.assertEqual(runtime.main(), code)
            self.assertEqual(out.getvalue(), '')
            self.assertIn(str(error), err.getvalue())


if __name__ == '__main__':
    unittest.main()
