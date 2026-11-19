"""TEST-001 A0：LTP 目录候选审计，不运行测例。"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import stat
import subprocess
import sys

import test001_audit as common

LOCK = common.ROOT / 'tests/test001/ltp-audit-inputs.lock.json'
INPUTS = common.ROOT / 'tests/test001/ltp-audit-inputs'


def parse_directory(raw):
    """只接受冻结 ls -p 格式；不推断未知文件类型或链接目标。"""
    if not raw or not raw.endswith(b'\n'):
        raise ValueError('directory listing must end with LF')
    lines = raw[:-1].decode('ascii').split('\n')
    # debugfs 在完整目录后输出一个空行；不接受中间空行或额外空记录。
    if lines and lines[-1] == '':
        lines.pop()
    rows, names = [], set()
    for number, line in enumerate(lines, 1):
        match = re.fullmatch(r'/([1-9][0-9]*)/([0-7]{6})/([0-9]+)/([0-9]+)/([A-Za-z0-9_.+-]+)/([0-9]*)/', line)
        if not match:
            raise ValueError(f'invalid directory record: {number}')
        inode, mode, uid, gid, name, size = match.groups()
        kind = stat.S_IFMT(int(mode, 8))
        if kind not in (stat.S_IFREG, stat.S_IFDIR):
            raise ValueError(f'unsupported directory member type: {name}')
        if name in names or (kind == stat.S_IFREG and (not size or name in ('.', '..'))):
            raise ValueError(f'duplicate or invalid directory member: {name}')
        names.add(name)
        rows.append({'name': name, 'inode': int(inode), 'mode': mode,
                     'uid': int(uid), 'gid': int(gid), 'size': int(size) if size else None,
                     'kind': 'regular' if kind == stat.S_IFREG else 'directory',
                     'listing_line': number})
    if not {'.', '..'}.issubset(names):
        raise ValueError('directory listing missing dot entries')
    return rows


def classify(rows):
    candidates, omitted = [], []
    # 仅报告展示顺序；不声称这是来宾 glob 顺序或观察调用顺序。
    for row in sorted(rows, key=lambda row: row['name'].encode('ascii')):
        reason = ('NOT_MATCHED_BY_GLOB' if row['name'].startswith('.') else
                  'NOT_REGULAR_FILE' if row['kind'] != 'regular' else None)
        if reason:
            omitted.append(dict(row, reason=reason))
        else:
            candidates.append(dict(row))
    return candidates, omitted


def audit(data, lock):
    groups = {}
    for libc in ('musl', 'glibc'):
        source = f'{libc}-ltp_testcode.sh'
        lines = data[source].decode('utf-8').splitlines()
        # 脚本已由固定锁逐字节核对；这是来源位置，不是通用 shell 解释器。
        call_lines = [i for i, line in enumerate(lines, 1) if line.strip() == '"$file"']
        if len(call_lines) != 1:
            raise ValueError('unsupported LTP invocation syntax')
        candidates, omitted = classify(parse_directory(data[f'{libc}-bin.debugfs']))
        for row in candidates:
            row.update(candidate_id=f'ltp:{libc}:{row["name"]}', cwd=f'/{libc}',
                       argv=[f'ltp/testcases/bin/{row["name"]}'],
                       script_file=source, script_line=call_lines[0],
                       listing_file=f'{libc}-bin.debugfs',
                       identity_status='BASENAME_ONLY_RUNTIME_PENDING')
        groups[f'ltp-{libc}'] = {'candidates': candidates, 'omitted': omitted,
                                 'expected_candidate_sequence': None,
                                 'presentation_order': 'ASCII_BASENAME_NOT_EXECUTION_ORDER'}
    names = {libc: {r['name'] for r in groups[f'ltp-{libc}']['candidates']}
             for libc in ('musl', 'glibc')}
    return {'schema_version': 1, 'status': 'AUDIT_CANDIDATE', 'runtime_ready': False,
            'image_verification': 'NOT_RUN', 'input_lock': lock, 'groups': groups,
            'libc_difference': {'musl_only': sorted(names['musl'] - names['glibc']),
                                'glibc_only': sorted(names['glibc'] - names['musl'])},
            'pending': ['guest shell/locale/glob options and invocation order',
                        'LTP build/source, per-file bytes, dependencies, resources and cleanup',
                        'native results grammar and samples, runtime markers, PROFILE/CRITERIA']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, default=INPUTS)
    parser.add_argument('--verify-image', type=Path)
    args = parser.parse_args()
    try:
        data, lock = common.load_inputs(args.inputs, LOCK)
        result = audit(data, lock)
        if args.verify_image:
            result['image_verification'] = common.verify_image(args.verify_image, lock)
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
        return 0
    except (ValueError, SyntaxError, FileNotFoundError) as error:
        print(f'audit input error: {error}', file=sys.stderr)
        return 2
    except (OSError, subprocess.SubprocessError) as error:
        print(f'audit execution error: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
