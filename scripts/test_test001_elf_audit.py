"""合成二进制仅测试身份检查和工具控制流，不作为 ELF 事实。"""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test001_audit as common
import test001_elf_audit as elf


class ElfAuditTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.lock = self.root / 'lock.json'
        self.lock.write_text(json.dumps({'executables': {'musl/lua': {
            'size': 3, 'sha256': common.digest(b'elf'), 'path': '/musl/lua'}}}))
        self.binary = self.root / 'musl-lua'
        self.binary.write_bytes(b'elf')
        self.addCleanup(patch.stopall)
        patch.object(common, 'LOCK', self.lock).start()

    def test_verified_private_copy_and_raw_evidence(self):
        def run(args, **kwargs):
            self.assertEqual(kwargs['env']['LC_ALL'], 'C')
            if '--version' not in args:
                private = Path(args[-1])
                self.assertNotEqual(private, self.binary)
                self.binary.write_bytes(b'bad')
                self.assertEqual(private.read_bytes(), b'elf')
            return subprocess.CompletedProcess(args, 0, b'raw evidence\n', b'')
        with patch.object(subprocess, 'run', side_effect=run):
            report = elf.inspect_binaries(self.root)
        self.assertFalse(report['runtime_ready'])
        self.assertEqual(report['image_verification'], 'NOT_RUN')
        self.assertEqual(report['executables']['musl/lua']['readelf_stdout'], 'raw evidence\n')

    def test_missing_short_changed_and_special_inputs_rejected_before_tools(self):
        for variant in ('missing', 'short', 'changed', 'directory'):
            with self.subTest(variant=variant):
                if self.binary.exists():
                    self.binary.unlink()
                if variant == 'short':
                    self.binary.write_bytes(b'e')
                elif variant == 'changed':
                    self.binary.write_bytes(b'bad')
                elif variant == 'directory':
                    self.binary.mkdir()
                with patch.object(subprocess, 'run') as run:
                    with self.assertRaises((ValueError, FileNotFoundError)):
                        elf.inspect_binaries(self.root)
                    run.assert_not_called()
                if self.binary.is_dir():
                    self.binary.rmdir()

    def test_all_inputs_checked_before_starting_tools(self):
        lock = json.loads(self.lock.read_text())
        lock['executables']['glibc/lua'] = lock['executables']['musl/lua']
        self.lock.write_text(json.dumps(lock))
        with patch.object(subprocess, 'run') as run:
            with self.assertRaises(FileNotFoundError):
                elf.inspect_binaries(self.root)
            run.assert_not_called()

    def test_cli_failures_never_print_partial_json(self):
        for error, expected in ((ValueError('input'), 2),
                                (subprocess.SubprocessError('tool'), 1)):
            out, err = io.StringIO(), io.StringIO()
            with patch('sys.argv', ['audit', '--binaries', str(self.root)]), \
                 patch.object(elf, 'inspect_binaries', side_effect=error), \
                 contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                self.assertEqual(elf.main(), expected)
            self.assertEqual(out.getvalue(), '')
            self.assertIn(str(error), err.getvalue())

    def test_tool_errors_and_diagnostics_rejected(self):
        for outcome in (subprocess.CalledProcessError(1, 'readelf', stderr=b'error'),
                        subprocess.TimeoutExpired('readelf', 30), OSError('missing readelf'),
                        subprocess.CompletedProcess([], 0, b'partial', b'warning'),
                        subprocess.CompletedProcess([], 0, b'\xff', b''),
                        subprocess.CompletedProcess([], 0, b'', b'')):
            with self.subTest(outcome=outcome), patch.object(subprocess, 'run') as run:
                if isinstance(outcome, Exception):
                    run.side_effect = outcome
                else:
                    run.return_value = outcome
                with self.assertRaises(subprocess.SubprocessError):
                    elf.inspect_binaries(self.root)


if __name__ == '__main__':
    unittest.main()
