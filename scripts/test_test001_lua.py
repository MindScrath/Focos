"""Lua 解析的手写期望；不使用生产词法生成测试期望。"""
from dataclasses import FrozenInstanceError
import json
from pathlib import Path
import unittest

from test001_lua import analyze_lua, LUA_NAMES

START = b'#### OS COMP TEST GROUP START lua-musl ####\n'
END = b'#### OS COMP TEST GROUP END lua-musl ####\n'
A = b'testcase lua date.lua success\n'
B = b'testcase lua file_io.lua success\n'
FAIL = b'testcase lua date.lua fail\n'


class LuaParserTests(unittest.TestCase):
    def codes(self, result):
        return {d.code.value for d in result.diagnostics}

    def parse(self, raw, names=('date.lua',)):
        try:
            result = analyze_lua(raw, 'musl', names)
        except RuntimeError as error:
            self.fail(f'log input must not break internal invariants: {error}')
        self.assertEqual(len(result.records), len(result.assignments))
        self.assertEqual([a.record_id for a in result.assignments], [r.record_id for r in result.records])
        self.assertEqual(b''.join(r.raw for r in result.records), raw)
        for r in result.records:
            self.assertEqual(raw[r.start:r.end], r.raw)
        self.assertEqual(result.acceptance, 'BLOCKED')
        self.assertEqual(result.trust, 'external_log/unknown')
        return result

    def test_plan_matches_frozen_source_order(self):
        root = Path(__file__).resolve().parents[1] / 'tests/test001/audit-inputs'
        for libc in ('musl', 'glibc'):
            lines = (root / libc / 'lua_testcode.sh').read_text().split('\n')
            actual = tuple(line[len('./test.sh '):] for line in lines if line.startswith('./test.sh '))
            self.assertEqual(actual, LUA_NAMES)

    def test_complete_and_failed_result(self):
        for row, judgment in ((A, 'PASS'), (FAIL, 'FAIL')):
            r = self.parse(START + row + END)
            self.assertEqual(r.protocol.value, 'COMPLETE')
            self.assertEqual(r.invocations[0].judgment.value, judgment)
            self.assertEqual(r.invocations[0].attempts[0].record_id, 2)
            self.assertEqual(r.assignments[1].owner, 'invocation')
            self.assertTrue(r.order_matches)

    def test_real_two_libc_groups(self):
        root = Path(__file__).resolve().parents[1] / 'tests/test001/fixtures/lua-a0'
        index = json.loads((root / 'index.json').read_text())
        for libc, sample in index['samples'].items():
            raw = (root / sample['serial_fragment']).read_bytes()
            begin, end = sample['group_byte_range_in_fragment']
            r = analyze_lua(raw[begin:end], libc)
            self.assertEqual(r.protocol.value, 'COMPLETE')
            self.assertEqual([i.name for i in r.invocations], list(LUA_NAMES))
            self.assertEqual([i.judgment.value for i in r.invocations], ['PASS'] * 9)
            whole = analyze_lua(raw, libc)
            self.assertIn('OUTSIDE_GROUP', self.codes(whole))
            self.assertEqual(len(whole.invocations), 9)

    def test_line_endings_and_sgr_mapping(self):
        for newline in (b'\n', b'\r\n'):
            raw = (START + b'\x1b[32m' + A[:-1] + b'\x1b[0m\n' + END).replace(b'\n', newline)
            r = self.parse(raw)
            self.assertEqual(r.protocol.value, 'COMPLETE')
            rec = r.records[1]
            self.assertEqual(rec.line, 2)
            self.assertEqual(rec.text, A[:-1].decode())
            self.assertEqual(bytes(raw[p] for p in rec.text_offsets), A[:-1])

    def test_invalid_structure_never_becomes_information(self):
        for bad in (b'\xc2\xa0' + A, A.replace(b' lua ', b'\xe2\x80\xa8lua '),
                    A.replace(b' lua ', b'\xc2\xa0lua '), A.replace(b' lua ', b'\vlua '),
                    A.replace(b' lua ', b'\flua '), A.replace(b' lua ', b'\rlua '),
                    A.replace(b' lua ', b'\xfflua '), b'\x1b[2K' + A,
                    A.replace(b'success', b'success junk'), A.replace(b'success', b'suc\ncess')):
            with self.subTest(bad=bad):
                r = self.parse(START + A + bad + END)
                self.assertEqual(r.protocol.value, 'ERROR')
                self.assertIn('INVALID_STRUCTURE', self.codes(r))
                self.assertEqual(r.assignments[2].owner, 'error')

    def test_message_does_not_create_result(self):
        r = self.parse(START + b'TINFO: quotes testcase lua date.lua success\n' + END)
        self.assertEqual(len(r.observations), 0)
        self.assertEqual(r.invocations[0].judgment.value, 'MISSING')
        self.assertEqual(r.assignments[1].owner, 'information')

    def test_zero_missing_and_truncation(self):
        empty = self.parse(b'')
        self.assertEqual(empty.protocol.value, 'INCOMPLETE')
        self.assertEqual(empty.invocations[0].judgment.value, 'NOT_RUN')
        self.assertIn('MISSING_GROUP', self.codes(empty))
        missing = self.parse(START + END)
        self.assertEqual(missing.invocations[0].judgment.value, 'MISSING')
        self.assertIn('ORDER_INCOMPLETE', self.codes(missing))
        truncated = self.parse(START + A)
        self.assertIn('TRUNCATED', self.codes(truncated))
        partial = self.parse(START + A + END[:-1])
        self.assertIn('INVALID_STRUCTURE', self.codes(partial))
        self.assertIn('TRUNCATED', self.codes(partial))

    def test_duplicate_preserves_failure(self):
        r = self.parse(START + FAIL + A + END)
        self.assertEqual(r.invocations[0].judgment.value, 'JUDGE_ERROR')
        self.assertEqual([a.status for a in r.invocations[0].attempts], ['fail', 'success'])
        self.assertIn('DUPLICATE_INVOCATION', self.codes(r))
        self.assertEqual(len(r.observed_sequence), 2)

    def test_order_and_selection_are_separate(self):
        good = self.parse(START + A + B + END, ('file_io.lua', 'date.lua'))
        self.assertEqual([i.name for i in good.invocations], ['date.lua', 'file_io.lua'])
        self.assertTrue(good.order_matches)
        bad = self.parse(START + B + A + END, ('date.lua', 'file_io.lua'))
        self.assertIn('ORDER_MISMATCH', self.codes(bad))
        self.assertFalse(bad.order_matches)
        outside = self.parse(START + B + END)
        self.assertIn('OUT_OF_SELECTION', self.codes(outside))
        self.assertEqual(len(outside.invocations), 1)
        self.assertEqual(outside.invocations[0].judgment.value, 'MISSING')
        self.assertEqual(outside.observations[0].name, 'file_io.lua')
        self.assertEqual(outside.observations[0].status, 'success')

    def test_unknown_and_orphan_never_fill_selection(self):
        for raw in (A + START + END, START + END + A):
            r = self.parse(raw)
            self.assertIn('OUTSIDE_GROUP', self.codes(r))
            self.assertEqual(r.invocations[0].judgment.value, 'MISSING')
            self.assertIsNone(r.observations[0].libc)
        r = self.parse(START + b'testcase lua alien.lua success\n' + END)
        self.assertIn('UNKNOWN_INVOCATION', self.codes(r))
        self.assertEqual(r.invocations[0].judgment.value, 'MISSING')

    def test_bad_markers_do_not_replace_active_group(self):
        r = self.parse(START + START.replace(b'lua-musl', b'lua-glibc') +
                       END.replace(b'lua-musl', b'busybox-musl') + A + END)
        self.assertEqual(r.protocol.value, 'ERROR')
        self.assertEqual(r.invocations[0].attempts[0].libc, 'musl')
        self.assertNotIn('TRUNCATED', self.codes(r))
        self.assertEqual([r.assignments[i].owner for i in (1, 2)], ['error', 'error'])

    def test_duplicate_groups_and_wrong_initial_group(self):
        duplicate = self.parse(START + A + END + START + END)
        self.assertIn('DUPLICATE_GROUP', self.codes(duplicate))
        wrong = self.parse(START.replace(b'lua-musl', b'lua-glibc') + A + END)
        self.assertIn('GROUP_MISMATCH', self.codes(wrong))
        self.assertEqual(wrong.invocations[0].judgment.value, 'NOT_RUN')
        self.assertIsNone(wrong.observations[0].libc)
        failed_outside = self.parse(START + B.replace(b'success', b'fail') + END)
        self.assertIn('OUT_OF_SELECTION', self.codes(failed_outside))
        self.assertEqual(failed_outside.observations[0].status, 'fail')
        self.assertEqual(failed_outside.invocations[0].judgment.value, 'MISSING')

    def test_reserved_markers_accept_ascii_field_whitespace(self):
        for fields in ((b'RUN', b'LTP', b'CASE'), (b'FAIL', b'LTP', b'CASE'),
                       (b'Total', b'Tests')):
            for separator in (b' ', b'\t', b'  ', b' \t ', b'\t\t'):
                marker = separator.join(fields) + b' access01'
                for line in (marker + b'\n', b' \t\x1b[31m' + marker + b'\x1b[0m\t\n'):
                    with self.subTest(fields=fields, separator=separator, line=line):
                        r = self.parse(START + A + line + END)
                        self.assertEqual(r.records[2].kind, 'unsupported')
                        self.assertEqual(r.assignments[2].owner, 'error')
                        self.assertIn('UNSUPPORTED_STRUCTURE', self.codes(r))
                        self.assertEqual(r.protocol.value, 'ERROR')
                        self.assertEqual(len(r.observations), 1)
                        self.assertEqual(r.invocations[0].judgment.value, 'PASS')
                partial = self.parse(START + A + END + marker)
                self.assertEqual(partial.records[-1].kind, 'invalid')
                self.assertIn('INVALID_STRUCTURE', self.codes(partial))
                quoted = self.parse(START + A + b'TINFO: quotes ' + marker + b'\n' + END)
                self.assertEqual(quoted.records[2].kind, 'information')
                self.assertEqual(quoted.protocol.value, 'COMPLETE')
            for separator in (b'\v', b'\f', '\u00a0'.encode()):
                r = self.parse(START + A + separator.join(fields) + b' access01\n' + END)
                self.assertEqual(r.records[2].kind, 'invalid')
                self.assertEqual(r.protocol.value, 'ERROR')

    def test_ltp_summary_without_colon_is_foreign(self):
        for field in (b'passed', b'failed', b'broken', b'skipped', b'warnings'):
            for separator in (b' ', b'  ', b'\t', b' \t '):
                for count in (b'0', b'1', b'999'):
                    line = field + separator + count + b'\n'
                    with self.subTest(line=line):
                        r = self.parse(START + A + line + END)
                        self.assertEqual(r.records[2].kind, 'unsupported')
                        self.assertEqual(r.records[2].raw, line)
                        self.assertEqual(r.assignments[2].owner, 'error')
                        self.assertIn('UNSUPPORTED_STRUCTURE', self.codes(r))
                        self.assertEqual(r.protocol.value, 'ERROR')
                        self.assertEqual(len(r.observations), 1)
                        self.assertEqual(r.invocations[0].judgment.value, 'PASS')
            for line in (b'\x1b[31m' + field + b'  1\x1b[0m\r\n', field + b': 1\n'):
                with self.subTest(line=line):
                    self.assertEqual(self.parse(START + A + line + END).records[2].kind, 'unsupported')
            with self.subTest(field=field, ending='no LF'):
                r = self.parse(START + A + END + field + b'  1')
                self.assertEqual(r.records[-1].kind, 'invalid')
                self.assertEqual(r.protocol.value, 'ERROR')
        for line in (b'failed to open file\n', b'passed through wrapper\n',
                     b'TINFO: failed  1\n', b'diagnostic: warnings 0\n', b'failedness 1\n'):
            with self.subTest(message=line):
                r = self.parse(START + A + line + END)
                self.assertEqual(r.records[2].kind, 'information')
                self.assertEqual(r.protocol.value, 'COMPLETE')

    def test_reserved_foreign_structures_block(self):
        for line in (b'failed: 999\n', b'Summary:\n', b'TPASS: fake\n',
                     b'testcase busybox true success\n', b'RUN LTP CASE access01\n'):
            r = self.parse(START + A + line + END)
            self.assertEqual(r.protocol.value, 'ERROR')
            self.assertEqual(r.assignments[2].owner, 'error')

    def test_ascii_whitespace_and_opaque_unicode(self):
        r = self.parse(START + '中文诊断'.encode() + b'\n' +
                       b' \ttestcase\tlua\tdate.lua\tsuccess \t\n' + END)
        self.assertEqual(r.protocol.value, 'COMPLETE')
        self.assertEqual(r.records[1].text, '中文诊断')
        self.assertEqual(r.assignments[1].owner, 'information')

    def test_reordered_single_calls_keep_result_and_ownership(self):
        for first, second, expected in ((B, A, ['PASS', 'PASS']),
                                         (B, FAIL, ['FAIL', 'PASS']),
                                         (B.replace(b'success', b'fail'), A, ['PASS', 'FAIL'])):
            with self.subTest(expected=expected):
                r = self.parse(START + first + second + END, ('date.lua', 'file_io.lua'))
                self.assertEqual(r.protocol.value, 'ERROR')
                self.assertIn('ORDER_MISMATCH', self.codes(r))
                self.assertEqual([i.judgment.value for i in r.invocations], expected)
                self.assertEqual([a.owner for a in r.assignments[1:3]], ['invocation', 'invocation'])
                self.assertEqual([a.rule for a in r.assignments[1:3]], ['lua_result', 'lua_result'])
                self.assertEqual([a.invocation for a in r.assignments[1:3]], ['file_io.lua', 'date.lua'])
                self.assertTrue(all(o.diagnostics for o in r.observations))

    def test_unicode_controls_and_formats_cannot_hide_records(self):
        controls = list(range(0x80, 0xa0)) + [0xad, 0x200b, 0x200e, 0x202e, 0x2066, 0x2069, 0xfeff, 0x13439, 0x1343f, 0xe0001, 0xe007f]
        for point in controls:
            control = chr(point).encode('utf-8')
            for line in (control + A, A[:-1] + control + b'\n', b'diagnostic ' + control + b'\n'):
                with self.subTest(point=hex(point), line=line):
                    r = self.parse(START + A + line + END)
                    self.assertEqual(r.protocol.value, 'ERROR')
                    self.assertIn('INVALID_STRUCTURE', self.codes(r))
                    self.assertEqual(r.assignments[2].owner, 'error')
                    self.assertEqual(r.records[2].raw, line)

    def test_order_incomplete_requires_exact_prefix(self):
        c = b'testcase lua max_min.lua success\n'
        names = ('date.lua', 'file_io.lua', 'max_min.lua')
        for body, missing in ((B, ['date.lua', 'max_min.lua']),
                              (A + c, ['file_io.lua']), (c, ['date.lua', 'file_io.lua'])):
            with self.subTest(body=body):
                r = self.parse(START + body + END, names)
                self.assertEqual(r.protocol.value, 'ERROR')
                self.assertIn('ORDER_MISMATCH', self.codes(r))
                self.assertNotIn('ORDER_INCOMPLETE', self.codes(r))
                self.assertEqual([i.name for i in r.invocations if i.judgment.value == 'MISSING'], missing)
        for body, next_name in ((b'', 'date.lua'), (A, 'file_io.lua'), (A + B, 'max_min.lua')):
            with self.subTest(body=body):
                r = self.parse(START + body + END, names)
                self.assertEqual(r.protocol.value, 'INCOMPLETE')
                diagnostics = [d for d in r.diagnostics if d.code.value == 'ORDER_INCOMPLETE']
                self.assertEqual(len(diagnostics), 1)
                self.assertEqual(diagnostics[0].expected, ('lua', 'musl', next_name))

    def test_damaged_prefix_preserves_unknown_start(self):
        prefixes = ('\u202e', '\u009b', '\u200b', '\ufeff', '\u00a0', '\u2028')
        prefixes = tuple(p.encode() for p in prefixes) + (
            b'\xff', b'\xe2\x82', b'\x00', b'\x1b[2K', b'\x1b[35m',
            b'\x1b]0;title\x07', b'\x1b[32m\xe2\x80\xae', b' \t\xff\xc2\xa0')
        for prefix in prefixes:
            with self.subTest(prefix=prefix):
                raw = START + prefix + A + END
                r = self.parse(raw)
                self.assertEqual(r.protocol.value, 'ERROR')
                self.assertIn('ORDER_UNVERIFIABLE', self.codes(r))
                self.assertNotIn('ORDER_INCOMPLETE', self.codes(r))
                self.assertEqual(r.observed_sequence, ((None, None, None),))
                self.assertEqual(len(r.observations), 1)
                observation = r.observations[0]
                self.assertEqual(observation.record_id, 2)
                self.assertIsNone(observation.name)
                self.assertIsNone(observation.status)
                self.assertFalse(observation.in_selection)
                self.assertEqual(r.invocations[0].judgment.value, 'MISSING')
                self.assertEqual(r.invocations[0].attempts, ())
                self.assertEqual(r.assignments[1].owner, 'error')
                self.assertEqual(r.records[1].raw, prefix + A)

    def test_damaged_prefix_does_not_search_inside_messages(self):
        lines = (b'TINFO: quotes ' + A, b'message: \xe2\x80\xae' + A,
                 b'\x1b]0;testcase lua date.lua success\x07 diagnostic\n',
                 b'\x1b]0;testcase lua date.lua success\n',
                 '\u009dtestcase lua date.lua success\u009c\n'.encode(),
                 b'literal \\xff' + A)
        for line in lines:
            with self.subTest(line=line):
                r = self.parse(START + line + END)
                self.assertEqual(r.observations, ())
                self.assertIn('ORDER_INCOMPLETE', self.codes(r))
                self.assertEqual(r.invocations[0].judgment.value, 'MISSING')
        combined = self.parse(START + A + b'\xe2\x80\xae' + A + END)
        self.assertEqual(combined.observed_sequence, (('lua', 'musl', 'date.lua'), (None, None, None)))
        self.assertEqual(len(combined.invocations[0].attempts), 1)
        self.assertEqual(combined.protocol.value, 'ERROR')

    def test_sgr_never_repairs_raw_utf8(self):
        colors = (b'\x1b[0m', b'\x1b[31m', b'\x1b[32m', b'\x1b[33m', b'\x1b[39m')
        for char in ('©', '中', '🧪'):
            encoded = char.encode()
            for split in range(1, len(encoded)):
                for color in colors:
                    with self.subTest(char=char, split=split, color=color):
                        line = encoded[:split] + color + encoded[split:] + A
                        r = self.parse(START + A + line + END)
                        self.assertEqual(r.protocol.value, 'ERROR')
                        self.assertEqual(r.records[2].kind, 'invalid')
                        self.assertIn(color.decode(), r.records[2].text)
                        self.assertIn('ORDER_UNVERIFIABLE', self.codes(r))
                        self.assertEqual(r.observed_sequence, (('lua', 'musl', 'date.lua'), (None, None, None)))
                        self.assertEqual(len(r.invocations[0].attempts), 1)
                        self.assertTrue(any(u.bad_byte is not None for u in r.records[2].units))
            for color in colors:
                for line in (color + encoded + A, encoded + color + A):
                    r = self.parse(START + A + line + END)
                    self.assertEqual(r.protocol.value, 'COMPLETE')
                    self.assertEqual(r.records[2].kind, 'information')
                    self.assertEqual(len(r.observations), 1)

    def test_raw_units_and_tokens_partition_without_loss(self):
        body = b'\x1b[32m' + A[:-1] + b'\x1b[0m\r\n'
        body += '中文 🧪\n'.encode() + b'\xc2\x1b[31m\xa9' + A
        raw = START + body + END
        r = self.parse(raw)
        for record in r.records:
            stop = record.end
            if raw[stop-1:stop] == b'\n':
                stop -= 1
                if raw[stop-1:stop] == b'\r':
                    stop -= 1
            position = record.start
            for unit in record.units:
                self.assertEqual(unit.start, position)
                fragment = raw[unit.start:unit.end]
                if unit.bad_byte is None:
                    self.assertEqual(fragment.decode('utf-8'), unit.text)
                    self.assertEqual(len(unit.text), 1)
                else:
                    self.assertIsNone(unit.text)
                    self.assertEqual(fragment, bytes([unit.bad_byte]))
                position = unit.end
            self.assertEqual(position, stop)
            cursor = 0
            for token in record.tokens:
                self.assertEqual(token.unit_start, cursor)
                self.assertGreater(token.unit_end, token.unit_start)
                self.assertEqual(token.start, record.units[token.unit_start].start)
                self.assertEqual(token.end, record.units[token.unit_end-1].end)
                cursor = token.unit_end
            self.assertEqual(cursor, len(record.units))

    def test_invalid_encodings_are_not_replaced_or_ignored(self):
        for bad in (b'\xff', b'\xc0\xaf', b'\xe0\x80\xaf', b'\xed\xa0\x80',
                    b'\xf4\x90\x80\x80', b'\xf0\x9f', b'\x80'):
            r = self.parse(START + bad + A + END)
            self.assertEqual(r.protocol.value, 'ERROR')
            self.assertEqual(r.observed_sequence, ((None, None, None),))
            self.assertEqual(r.invocations[0].attempts, ())
            self.assertEqual([u.bad_byte for u in r.records[1].units if u.bad_byte is not None], list(bad))

    def test_control_strings_are_tokenized_before_color_removal(self):
        payload = b'0;\x1b[31mtestcase lua date.lua success\x1b[0m'
        for prefix, suffix in ((b'\x1b]', b'\x07'), (b'\x1b]', b'\x1b\\'),
                               ('\u009d'.encode(), '\u009c'.encode())):
            for ending in (suffix + b' diagnostic\n', b'\n'):
                r = self.parse(START + prefix + payload + ending + END)
                self.assertEqual(r.protocol.value, 'ERROR')
                self.assertEqual(r.observations, ())
                self.assertFalse(any(t.kind == 'sgr' for t in r.records[1].tokens))
                self.assertEqual(r.records[1].text, (prefix + payload + ending)[:-1].decode())
            r = self.parse(START + prefix + payload + suffix + A + END)
            self.assertEqual(r.observed_sequence, ((None, None, None),))
        for line in (b'\x1bPtestcase lua date.lua success\x1b\\\n',
                     b'\x1b[31' + b'\n', b'\x1b]0;\xfftestcase lua date.lua success\x07\n'):
            r = self.parse(START + line + END)
            self.assertEqual(r.protocol.value, 'ERROR')
            self.assertEqual(r.observations, ())

    def test_corruption_keeps_scope_order_and_failure_facts(self):
        bad = b'\xc2\x1b[31m\xa9' + A
        outside = self.parse(START + B + bad + END)
        self.assertIn('OUT_OF_SELECTION', self.codes(outside))
        self.assertIn('ORDER_UNVERIFIABLE', self.codes(outside))
        self.assertEqual(outside.invocations[0].judgment.value, 'MISSING')
        repeated = self.parse(START + FAIL + A + bad + END)
        self.assertEqual([a.status for a in repeated.invocations[0].attempts], ['fail', 'success'])
        self.assertEqual(repeated.invocations[0].judgment.value, 'JUDGE_ERROR')
        self.assertEqual(len(repeated.observations), 3)
        reordered = self.parse(START + B + bad + A + END, ('date.lua', 'file_io.lua'))
        self.assertIn('ORDER_MISMATCH', self.codes(reordered))
        self.assertEqual([i.judgment.value for i in reordered.invocations], ['PASS', 'PASS'])
        before = self.parse(bad + START + A + END)
        self.assertEqual(before.observed_sequence, ((None, None, None), ('lua', 'musl', 'date.lua')))
        self.assertEqual(before.assignments[0].owner, 'error')
        self.assertIsNone(before.assignments[0].group)

    def test_immutable_and_invalid_selection(self):
        r = self.parse(START + A + END)
        with self.assertRaises(FrozenInstanceError):
            r.acceptance = 'PASS'
        for names in ((), ('alien.lua',), ('date.lua', 'date.lua')):
            with self.assertRaises(ValueError):
                analyze_lua(b'', 'musl', names)
        with self.assertRaises(ValueError):
            analyze_lua(b'', 'other')
        with self.assertRaises(TypeError):
            analyze_lua(bytearray(), 'musl')


if __name__ == '__main__':
    unittest.main()
