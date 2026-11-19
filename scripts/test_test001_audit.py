"""A0 审计负例：不执行镜像命令或官方 judge。"""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from test001_audit import audit, read_command_lines, judge_commands, load_inputs, verify_image

ROOT = Path(__file__).resolve().parents[1]


class AuditTests(unittest.TestCase):
    def test_read_semantics_preserves_raw_and_duplicate(self):
        rows = read_command_lines(b' date \n echo "x"\necho "x"\nprintf "abc\\n"\n')
        self.assertEqual([r['line'] for r in rows], [1, 2, 3, 4])
        self.assertEqual([r['command'] for r in rows],
                         ['date', 'echo "x"', 'echo "x"', 'printf "abcn"'])
        self.assertEqual(rows[0]['raw'], ' date ')

    def test_read_matches_real_shell_without_eval(self):
        raw = (ROOT / 'tests/test001/audit-inputs/musl/busybox_cmd.txt').read_bytes()
        result = subprocess.run(['/bin/sh', '-c',
                                 'while read line; do printf "%s\\n" "$line"; done'],
                                input=raw, capture_output=True, check=True)
        self.assertEqual([r['command'] for r in read_command_lines(raw)],
                         result.stdout.decode().splitlines())

    def test_unsupported_command_records_reject(self):
        for raw in [b'date', b'\n', b'date\r\n', b'echo a\\\nb\n', b'date\x00\n', b'echo a\\ \n', b'echo a\\ b\n']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                read_command_lines(raw)

    def test_judge_is_parsed_never_executed(self):
        raw = b'raise RuntimeError("must not execute")\ncmds="a\\nb\\na"\n'
        self.assertEqual(judge_commands(raw), ['a', 'b', 'a'])
        for raw in [b'cmds = get_commands()', b'cmds="a"\ncmds="b"']:
            with self.assertRaises(ValueError):
                judge_commands(raw)

    def test_real_inventory_counts_and_busybox_differences(self):
        data, lock = load_inputs(ROOT / 'tests/test001/audit-inputs')
        result = audit(data, lock)
        self.assertEqual(result['status'], 'AUDIT_CANDIDATE')
        for libc in ['musl', 'glibc']:
            busy = result['groups'][f'busybox-{libc}']
            self.assertEqual(len(busy['calls']), 55)
            self.assertEqual(busy['unique_commands'], 54)
            self.assertEqual(busy['judge_unique'], 53)
            self.assertEqual(busy['judge_only'], ['kill 10'])
            self.assertEqual(busy['image_only_raw'],
                             ['printf "abc\\n"', "sh -c 'sleep 5' & ./busybox kill $!"])
            duplicates = [c for c in busy['calls'] if c['command'] == 'echo "bbbbbbb" >> test.txt']
            self.assertEqual(len(duplicates), 2)
            self.assertNotEqual(duplicates[0]['candidate_id'], duplicates[1]['candidate_id'])
            lua = result['groups'][f'lua-{libc}']
            self.assertEqual(len(lua['calls']), 9)
            self.assertEqual(lua['image_only_raw'], [])
            self.assertEqual(lua['judge_only'], [])
            self.assertEqual(lua['calls'][0]['argv'], ['./test.sh', 'date.lua'])

    def test_hash_mismatch_and_extra_input_rejected(self):
        import shutil
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'inputs'
            shutil.copytree(ROOT / 'tests/test001/audit-inputs', root)
            (root / 'musl/busybox_cmd.txt').write_bytes(b'true\n')
            with self.assertRaises(ValueError):
                load_inputs(root)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'inputs'
            shutil.copytree(ROOT / 'tests/test001/audit-inputs', root)
            (root / 'extra').write_bytes(b'')
            with self.assertRaises(ValueError):
                load_inputs(root)

    def test_symlink_input_rejected(self):
        import shutil
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'inputs'
            shutil.copytree(ROOT / 'tests/test001/audit-inputs', root)
            source = root / 'musl/busybox_cmd.txt'
            source.unlink()
            source.symlink_to(ROOT / 'tests/test001/audit-inputs/musl/busybox_cmd.txt')
            with self.assertRaises(ValueError):
                load_inputs(root)

    def test_checked_in_candidate_table(self):
        import json
        data, lock = load_inputs(ROOT / 'tests/test001/audit-inputs')
        result = audit(data, lock)
        rows = [dict(group_libc=key, **call)
                for key, group in sorted(result['groups'].items()) for call in group['calls']]
        actual = [json.loads(line) for line in
                  (ROOT / 'tests/test001/audit-calls.jsonl').read_text().splitlines()]
        self.assertEqual(actual, rows)

    def test_bad_image_rejected_before_debugfs(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            image.write_bytes(b'bad image')
            _, lock = load_inputs(ROOT / 'tests/test001/audit-inputs')
            with patch('test001_audit.subprocess.run') as run:
                with self.assertRaises(ValueError):
                    verify_image(image, lock)
                run.assert_not_called()

    def test_invalid_image_rejected_before_copy(self):
        from unittest.mock import patch
        from test001_audit import copy_image_snapshot
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            _, lock = load_inputs(ROOT / 'tests/test001/audit-inputs')
            with image.open('wb') as stream:
                stream.truncate(lock['image']['uncompressed_size'] + 1)
            for source in [image, Path('/dev/zero'), Path(tmp)]:
                with self.subTest(source=source), \
                     patch('test001_audit.shutil.copyfile', side_effect=AssertionError('unbounded copy')), \
                     patch('test001_audit.subprocess.run') as run:
                    with self.assertRaises(ValueError):
                        verify_image(source, lock)
                    run.assert_not_called()
                    snapshot = Path(tmp) / 'rejected-snapshot'
                    with self.assertRaises(ValueError):
                        copy_image_snapshot(source, snapshot, lock['image']['uncompressed_size'])
                    self.assertFalse(snapshot.exists())

    def test_snapshot_growth_and_truncation_bounded(self):
        import os
        from unittest.mock import patch
        from test001_audit import copy_image_snapshot
        real_fstat = os.fstat
        with tempfile.TemporaryDirectory() as tmp:
            image, snapshot = Path(tmp) / 'image', Path(tmp) / 'snapshot'
            for replacement, message in [(b'goodEXTRA', 'grew'), (b'go', 'truncated')]:
                image.write_bytes(b'good')
                snapshot.unlink(missing_ok=True)
                opened = []

                def change_after_stat(fd):
                    opened.append(fd)
                    info = real_fstat(fd)
                    image.write_bytes(replacement)
                    return info

                with self.subTest(replacement=replacement), \
                     patch('test001_audit.os.fstat', side_effect=change_after_stat):
                    with self.assertRaisesRegex(ValueError, message):
                        copy_image_snapshot(image, snapshot, 4)
                self.assertLessEqual(snapshot.stat().st_size, 4)
                with self.assertRaises(OSError):
                    real_fstat(opened[0])

    def test_snapshot_reads_open_file_after_path_replacement(self):
        import os
        from unittest.mock import patch
        from test001_audit import copy_image_snapshot
        real_fstat = os.fstat
        with tempfile.TemporaryDirectory() as tmp:
            image, snapshot = Path(tmp) / 'image', Path(tmp) / 'snapshot'
            image.write_bytes(b'good')

            def replace_after_stat(fd):
                info = real_fstat(fd)
                image.rename(Path(tmp) / 'original')
                image.write_bytes(b'evil')
                return info

            with patch('test001_audit.os.fstat', side_effect=replace_after_stat):
                copy_image_snapshot(image, snapshot, 4)
            self.assertEqual(snapshot.read_bytes(), b'good')
            self.assertEqual(image.read_bytes(), b'evil')

    def test_cli_rejects_fifo_without_waiting(self):
        import os
        with tempfile.TemporaryDirectory() as tmp:
            fifo = Path(tmp) / 'image-fifo'
            os.mkfifo(fifo)
            result = subprocess.run([sys.executable, '-B', str(ROOT / 'scripts/test001_audit.py'),
                                     '--verify-image', str(fifo)], capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b'')
            self.assertIn(b'regular file', result.stderr)

    def test_image_hash_without_python311_api(self):
        import hashlib
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            image.write_bytes(b'not the frozen image')
            _, lock = load_inputs(ROOT / 'tests/test001/audit-inputs')
            # 保持长度匹配，确保确实走到哈希步骤，不被新增预检提前拒绝。
            lock['image']['uncompressed_size'] = image.stat().st_size
            # 模拟 Python 3.10 不提供 file_digest；仍须正常识别坏镜像。
            with patch.dict(hashlib.__dict__):
                hashlib.__dict__.pop('file_digest', None)
                with self.assertRaisesRegex(ValueError, 'image identity mismatch'):
                    verify_image(image, lock)

    def test_debugfs_member_errors_are_execution_errors(self):
        import contextlib
        import hashlib
        import io
        from unittest.mock import patch
        from test001_audit import main
        data, _ = load_inputs(ROOT / 'tests/test001/audit-inputs')
        # 合成镜像用于 host 控制流测试，不是 ext4 或真实采集证据。
        image_bytes = b'synthetic image'
        member_bytes = b'abc'
        lock = {
            'image': {'uncompressed_size': len(image_bytes),
                      'uncompressed_sha256': hashlib.sha256(image_bytes).hexdigest()},
            'files': {'musl/member': {'path': '/musl/member', 'source': 'image',
                                    'size': 3, 'sha256': hashlib.sha256(member_bytes).hexdigest()}},
            'executables': {},
        }
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / 'image'
            image.write_bytes(image_bytes)
            for stdout, stderr, failure in [
                (b'', b'File not found by ext2_lookup', None),
                (b'ab', b'short read', None),
                (b'xyz', b'wrong bytes', None),
                (b'', b'read error', subprocess.CalledProcessError(1, 'debugfs', stderr=b'read error')),
                (b'', b'timed out', subprocess.TimeoutExpired('debugfs', 30, stderr=b'timed out')),
                (b'', b'tool vanished', FileNotFoundError('tool vanished')),
            ]:
                with self.subTest(stdout=stdout, failure=failure):
                    out, err = io.StringIO(), io.StringIO()
                    response = subprocess.CompletedProcess(['debugfs'], 0, stdout, stderr)
                    with patch('test001_audit.load_inputs', return_value=(data, lock)), \
                         patch('test001_audit.shutil.which', return_value='/usr/bin/debugfs'), \
                         patch('test001_audit.subprocess.run', return_value=response, side_effect=failure), \
                         patch.object(sys, 'argv', ['audit', '--verify-image', str(image)]), \
                         contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                        code = main()
                    self.assertEqual(code, 1)
                    self.assertEqual(out.getvalue(), '')
                    self.assertIn('musl/member', err.getvalue())
                    self.assertIn(stderr.decode(), err.getvalue())

    def test_debugfs_version_start_failure_is_execution_error(self):
        from unittest.mock import patch
        from test001_audit import run_debugfs
        with patch('test001_audit.subprocess.run', side_effect=FileNotFoundError('missing')):
            with self.assertRaisesRegex(subprocess.SubprocessError, 'version query'):
                run_debugfs(['-V'], 'version query', 10)

    def test_cli_bad_input_has_no_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run([sys.executable, '-B', str(ROOT / 'scripts/test001_audit.py'),
                                     '--inputs', tmp], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b'')

    def test_cli_deterministic_no_execution_claim(self):
        cmd = [sys.executable, '-B', str(ROOT / 'scripts/test001_audit.py')]
        first = subprocess.run(cmd, capture_output=True, check=True)
        second = subprocess.run(cmd, capture_output=True, check=True)
        self.assertEqual(first.stdout, second.stdout)
        import json
        result = json.loads(first.stdout)
        self.assertEqual(result['image_verification'], 'NOT_RUN')
        self.assertFalse(result['runtime_ready'])


if __name__ == '__main__':
    unittest.main()
