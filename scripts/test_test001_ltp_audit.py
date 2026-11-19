"""LTP A0：合成目录只验证枚举，不执行来宾用例。"""
import contextlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test001_audit as common
import test001_ltp_audit as ltp

HEADER = b'/1/040755/0/0/.//\n/2/040755/0/0/..//\n'


class LtpAuditTests(unittest.TestCase):
    def test_frozen_inventory_and_libc_difference(self):
        data, lock = common.load_inputs(ltp.INPUTS, ltp.LOCK)
        result = ltp.audit(data, lock)
        musl, glibc = (result['groups']['ltp-' + c] for c in ('musl', 'glibc'))
        self.assertEqual([len(g['candidates']) for g in (musl, glibc)], [2820, 2840])
        self.assertFalse(result['runtime_ready'])
        self.assertIsNone(musl['expected_candidate_sequence'])
        self.assertEqual({r['name'] for r in musl['omitted']}, {'.', '..', 'datafiles', 'ftrace_stress'})
        self.assertEqual(len(result['libc_difference']['glibc_only']), 20)
        self.assertEqual(result['libc_difference']['musl_only'], [])
        self.assertTrue(all(r['argv'] == ['ltp/testcases/bin/' + r['name']] for r in musl['candidates']))
        self.assertTrue(all('execution_index' not in r for r in musl['candidates']))

    def test_file_types_hidden_and_nonexecutable(self):
        rows = ltp.parse_directory(HEADER + b'/3/100644/0/0/plain/2/\n/4/100755/0/0/.hidden/1/\n/5/040755/0/0/sub//\n')
        candidates, omitted = ltp.classify(rows)
        self.assertEqual([r['name'] for r in candidates], ['plain'])
        self.assertEqual({r['name']: r['reason'] for r in omitted},
                         {'.': 'NOT_MATCHED_BY_GLOB', '..': 'NOT_MATCHED_BY_GLOB',
                          '.hidden': 'NOT_MATCHED_BY_GLOB', 'sub': 'NOT_REGULAR_FILE'})

    def test_listing_order_is_not_execution_order(self):
        rows = ltp.parse_directory(HEADER + b'/3/100755/0/0/z/1/\n/4/100755/0/0/a/1/\n')
        candidates, _ = ltp.classify(rows)
        self.assertEqual([r['name'] for r in candidates], ['a', 'z'])
        self.assertEqual([r['listing_line'] for r in candidates], [4, 3])

    def test_empty_directory_has_no_invocations(self):
        self.assertEqual(ltp.classify(ltp.parse_directory(HEADER))[0], [])

    def test_malformed_duplicate_and_unsupported_types(self):
        for raw in (b'', HEADER[:-1], HEADER.replace(b'\n', b'\r\n'), HEADER + b'junk\n',
                    HEADER + b'/3/120777/0/0/link/4/\n',
                    HEADER + b'/3/010644/0/0/fifo/0/\n',
                    HEADER + b'/3/100755/0/0/../4/\n',
                    HEADER + b'/3/100755/0/0/a//\n',
                    HEADER + b'/3/100755/0/0/a b/1/\n',
                    HEADER + b'/3/100755/0/0/a/1/\n/4/100755/0/0/a/2/\n',
                    HEADER.replace(b'/.//', b'/bad//'),
                    HEADER + b'\n/3/100755/0/0/a/1/\n'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                ltp.parse_directory(raw)

    def test_modified_and_extra_inputs_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'inputs'
            shutil.copytree(ltp.INPUTS, root)
            (root / 'extra').write_bytes(b'')
            with self.assertRaises(ValueError):
                common.load_inputs(root, ltp.LOCK)
            (root / 'extra').unlink()
            with (root / 'musl-ltp_testcode.sh').open('ab') as stream:
                stream.write(b'\n')
            with self.assertRaises(ValueError):
                common.load_inputs(root, ltp.LOCK)

    def test_cli_tool_failure_has_no_json(self):
        out, err = io.StringIO(), io.StringIO()
        with patch('sys.argv', ['audit', '--verify-image', '/synthetic']), \
             patch.object(common, 'verify_image', side_effect=subprocess.SubprocessError('read failed')), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(ltp.main(), 1)
        self.assertEqual(out.getvalue(), '')
        self.assertIn('read failed', err.getvalue())

    def test_cli_invalid_inputs_has_no_json(self):
        out, err = io.StringIO(), io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, \
             patch('sys.argv', ['audit', '--inputs', tmp]), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(ltp.main(), 2)
        self.assertEqual(out.getvalue(), '')
        self.assertIn('input file set', err.getvalue())

    def test_candidate_table_regenerates(self):
        data, lock = common.load_inputs(ltp.INPUTS, ltp.LOCK)
        report = ltp.audit(data, lock)
        text = ''.join(json.dumps(dict(group_libc=key, **row), sort_keys=True) + '\n'
                       for key, group in sorted(report['groups'].items()) for row in group['candidates'])
        self.assertEqual(text, (common.ROOT / 'tests/test001/ltp-audit-candidates.jsonl').read_text())

    def test_debugfs_directory_operation_and_zero_exit_failure(self):
        raw = b'synthetic image'
        lock = {'image': {'uncompressed_size': len(raw), 'uncompressed_sha256': common.digest(raw)},
                'files': {'listing': {'source': 'image', 'path': '/musl/ltp/testcases/bin',
                                      'operation': 'ls -p', 'size': len(HEADER), 'sha256': common.digest(HEADER)}},
                'executables': {}}
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            image.write_bytes(raw)
            with patch.object(common.shutil, 'which', return_value='/debugfs'), \
                 patch.object(common, 'run_debugfs', side_effect=[subprocess.CompletedProcess([], 0, HEADER, b''),
                                                                subprocess.CompletedProcess([], 0, b'', b'version')]) as run:
                self.assertEqual(common.verify_image(image, lock)['members'], 1)
                self.assertEqual(run.call_args_list[0].args[0][1], 'ls -p /musl/ltp/testcases/bin')
            lock['files']['listing']['operation'] = 'dump'
            with patch.object(common.shutil, 'which', return_value='/debugfs'), \
                 patch.object(common, 'run_debugfs') as run:
                with self.assertRaisesRegex(ValueError, 'unsupported locked image operation'):
                    common.verify_image(image, lock)
                run.assert_not_called()
            lock['files']['listing']['operation'] = 'ls -p'
            with patch.object(common.shutil, 'which', return_value='/debugfs'), \
                 patch.object(common, 'run_debugfs', return_value=subprocess.CompletedProcess([], 0, b'', b'not found')):
                with self.assertRaisesRegex(subprocess.SubprocessError, 'listing.*not found'):
                    common.verify_image(image, lock)


if __name__ == '__main__':
    unittest.main()
