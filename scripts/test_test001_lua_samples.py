"""校验真实 Lua 语法样本的来源和包装器观察；不是运行判分器。"""
import hashlib
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'tests/test001/fixtures/lua-a0'


class LuaSampleTests(unittest.TestCase):
    def test_evidence_bytes_and_input_identity(self):
        index = json.loads((FIXTURES / 'index.json').read_text())
        lock = json.loads((ROOT / 'tests/test001/audit-inputs.lock.json').read_text())
        self.assertEqual(index['scope'], 'A0_SYNTAX_SAMPLE_NOT_CAPTURE')
        for name, meta in index['evidence_files'].items():
            raw = (FIXTURES / name).read_bytes()
            self.assertEqual(len(raw), meta['size'])
            self.assertEqual(hashlib.sha256(raw).hexdigest(), meta['sha256'])
        for libc, sample in index['samples'].items():
            members = {m['path']: m for m in sample['archive_members']}
            lua_names = ('date', 'file_io', 'max_min', 'random', 'remove', 'round_num', 'sin30', 'sort', 'strings')
            expected = {'bin', 'dev', libc, 'dev/console', 'dev/null', 'bin/busybox', 'bin/sh', 'init'}
            expected.update(f'{libc}/{name}' for name in ('busybox', 'lua', 'test.sh', 'lua_testcode.sh'))
            expected.update(f'{libc}/{name}.lua' for name in lua_names)
            self.assertEqual(set(members), expected)
            self.assertEqual(len(members), len(sample['archive_members']))
            begin, end = sample['fragment_byte_range']
            self.assertGreaterEqual(begin, 0)
            self.assertLessEqual(end, sample['full_stdout']['size'])
            self.assertEqual(end - begin, index['evidence_files'][sample['serial_fragment']]['size'])
            for name, meta in {**lock['files'], **lock['executables']}.items():
                if name in members:
                    self.assertEqual(members[name]['sha256'], meta['sha256'])
                    self.assertEqual(members[name]['size'], meta['size'])
            self.assertEqual(members['bin/busybox']['sha256'],
                             hashlib.sha256(('../' + libc + '/busybox').encode()).hexdigest())
            self.assertEqual(members['init']['sha256'], index['evidence_files']['init.sh']['sha256'])
            self.assertEqual(sample['qemu_returncode'], 0)
            self.assertEqual(sample['stderr_size'], 0)

    def test_real_group_order_and_process_probes(self):
        index = json.loads((FIXTURES / 'index.json').read_text())
        names = ('date', 'file_io', 'max_min', 'random', 'remove', 'round_num', 'sin30', 'sort', 'strings')
        for libc in ('musl', 'glibc'):
            raw = (FIXTURES / (libc + '.serial.fragment')).read_bytes()
            start = f'#### OS COMP TEST GROUP START lua-{libc} ####\r\n'.encode()
            end = f'#### OS COMP TEST GROUP END lua-{libc} ####\r\n'.encode()
            group = start + b''.join(f'testcase lua {name}.lua success\r\n'.encode() for name in names) + end
            begin, finish = index['samples'][libc]['group_byte_range_in_fragment']
            self.assertEqual(raw[begin:finish], group)
            self.assertEqual(raw.count(start), 1)
            self.assertEqual(raw.count(end), 1)
            self.assertIn(b'A0_RETURN_MINUS_ONE_RC 0\r\n', raw)
            self.assertIn(b'A0_EXPLICIT_ERROR_RC 1\r\n', raw)
            self.assertIn(b'testcase lua __a0_missing.lua fail\r\nA0_WRAPPER_MISSING_INPUT_RC 0\r\n', raw)
            self.assertIn(b'A0_POST_TEST_TXT ABSENT\r\n', raw)


if __name__ == '__main__':
    unittest.main()
