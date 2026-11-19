"""TEST-001 A0：审计固定镜像的 BusyBox/Lua 输入，不运行测例或 judge。"""
from __future__ import annotations

import argparse
import ast
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import stat
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / 'tests/test001/audit-inputs.lock.json'
INPUTS = ROOT / 'tests/test001/audit-inputs'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def check_bytes(raw, meta, name):
    if len(raw) != meta['size'] or digest(raw) != meta['sha256']:
        raise ValueError(f'input identity mismatch: {name}')


def load_inputs(root, lock_path=LOCK):
    # 读取后只消费内存副本，不在审计中重开路径。
    lock = json.loads(lock_path.read_bytes())
    root = Path(root)
    if root.is_symlink():
        raise ValueError('input root is a symlink')
    paths = list(root.rglob('*'))
    if any(p.is_symlink() or not (p.is_file() or p.is_dir()) for p in paths):
        raise ValueError('input contains a link or special file')
    actual = {p.relative_to(root).as_posix() for p in paths if p.is_file()}
    if actual != set(lock['files']):
        raise ValueError('input file set differs from frozen lock')
    data = {}
    for name, meta in lock['files'].items():
        raw = (root / name).read_bytes()
        check_bytes(raw, meta, name)
        data[name] = raw
    return data, lock


def read_command_lines(raw):
    """仅覆盖锁定表使用的单物理行 read（默认 IFS，无 -r）。"""
    if not raw or not raw.endswith(b'\n'):
        raise ValueError('command table must end with LF')
    rows = []
    for number, line in enumerate(raw[:-1].decode('utf-8').split('\n'), 1):
        if not line or any(ord(c) < 32 and c != '\t' for c in line):
            raise ValueError('unsupported empty/control record')
        # 本锁无转义 IFS 空白；不模拟不同 shell 的字段分割细节。
        chars = []
        pos = 0
        while pos < len(line):
            escaped = line[pos] == '\\'
            if escaped:
                pos += 1
                if pos == len(line):
                    raise ValueError('continued read records are not supported')
                if line[pos] in ' \t':
                    raise ValueError('escaped IFS whitespace is not supported')
            chars.append((line[pos], escaped))
            pos += 1
        while chars and chars[0][0] in ' \t' and not chars[0][1]:
            chars.pop(0)
        while chars and chars[-1][0] in ' \t' and not chars[-1][1]:
            chars.pop()
        command = ''.join(c for c, _ in chars)
        if not command:
            raise ValueError('empty command after read')
        rows.append({'line': number, 'raw': line, 'command': command})
    return rows


def judge_commands(raw):
    # AST 只读取常量，不 import/exec/eval 上游程序。
    tree = ast.parse(raw)
    assignments = [n for n in tree.body if isinstance(n, ast.Assign)
                   and any(isinstance(t, ast.Name) and t.id == 'cmds' for t in n.targets)]
    if len(assignments) != 1:
        raise ValueError('judge must contain one literal cmds assignment')
    node = assignments[0].value
    if not isinstance(node, ast.Constant) or not isinstance(node.value, str):
        raise ValueError('judge cmds must be a string literal')
    return [line.strip() for line in node.value.split('\n') if line.strip()]


def differences(calls, baseline):
    raw_names = [c['raw'].strip() for c in calls]
    return {
        'unique_commands': len(set(raw_names)),
        'judge_rows': len(baseline),
        'judge_unique': len(set(baseline)),
        'image_only_raw': sorted(set(raw_names) - set(baseline)),
        'judge_only': sorted(set(baseline) - set(raw_names)),
        'planned_repetitions': {k: v for k, v in Counter(raw_names).items() if v > 1},
    }


def audit(data, lock):
    groups = {}
    for libc in ('musl', 'glibc'):
        for group in ('busybox', 'lua'):
            source = f'{libc}/{group}_testcode.sh'
            calls = []
            if group == 'busybox':
                rows = read_command_lines(data[f'{libc}/busybox_cmd.txt'])
                for row in rows:
                    calls.append({**row, 'candidate_id': f'{group}:{libc}:line-{row["line"]:03}',
                                  'source_file': f'{libc}/busybox_cmd.txt',
                                  'cwd': f'/{libc}', 'argv': None,
                                  'eval_source': './busybox ' + row['command'],
                                  'script_file': source,
                                  'identity_status': 'NEEDS_RUNTIME_MARKER'})
            else:
                for number, line in enumerate(data[source].decode().splitlines(), 1):
                    if not line.startswith('./test.sh '):
                        continue
                    match = re.fullmatch(r'\./test\.sh ([a-z0-9_]+\.lua)', line)
                    if not match:
                        raise ValueError('unsupported Lua invocation syntax')
                    name = match[1]
                    if f'{libc}/{name}' not in data:
                        raise ValueError(f'missing Lua input: {name}')
                    calls.append({'candidate_id': f'{group}:{libc}:line-{number:03}',
                                  'source_file': source, 'line': number, 'raw': name,
                                  'source_line': line, 'command': name,
                                  'cwd': f'/{libc}', 'argv': ['./test.sh', name],
                                  'wrapped_argv': ['./lua', name],
                                  'input_file': f'{libc}/{name}',
                                  'identity_status': 'UNIQUE_ARGUMENT'})
            baseline = judge_commands(data[f'judge/judge_{group}.py'])
            groups[f'{group}-{libc}'] = {
                'calls': calls, **differences(calls, baseline),
                'expected_candidate_sequence': [c['candidate_id'] for c in calls],
            }
    return {
        'schema_version': 1, 'status': 'AUDIT_CANDIDATE', 'runtime_ready': False,
        'image_verification': 'NOT_RUN', 'input_lock': lock,
        'groups': groups,
        'pending': ['runtime environment, ELF dependencies and resource policy',
                    'BusyBox start/end IDs and cleanup/precondition instrumentation',
                    'remaining groups including LTP; no executable PROFILE/CRITERIA'],
    }


def run_debugfs(arguments, context, timeout):
    try:
        return subprocess.run(['debugfs', *arguments], capture_output=True,
                              timeout=timeout, check=True)
    except (OSError, subprocess.SubprocessError) as error:
        diagnostic = getattr(error, 'stderr', None) or b''
        if isinstance(diagnostic, bytes):
            diagnostic = diagnostic.decode('utf-8', errors='replace')
        raise subprocess.SubprocessError(
            f'debugfs {context} failed: {error}; {diagnostic}'
        ) from error


def copy_image_snapshot(image, snapshot, expected_size):
    # 非阻塞打开避免 FIFO 在 fstat 之前等待写端；普通文件不受此标志影响。
    descriptor = os.open(image, os.O_RDONLY | os.O_NONBLOCK)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise ValueError('image source must be a regular file')
        if info.st_size != expected_size:
            raise ValueError('image identity mismatch: unexpected source size')
        # 只使用上述句柄，不按可能被替换的原路径重开。增长最多多读一字节，绝不多写。
        with os.fdopen(descriptor, 'rb', buffering=0, closefd=False) as source, snapshot.open('xb') as target:
            remaining = expected_size
            while remaining:
                chunk = source.read(min(1024 * 1024, remaining))
                if not chunk:
                    raise ValueError('image source truncated during copy')
                target.write(chunk)
                remaining -= len(chunk)
            if source.read(1):
                raise ValueError('image source grew during copy')
    finally:
        os.close(descriptor)


def verify_image(image, lock):
    # 整盘先复制到私有目录再核对实际消费字节；debugfs 不带 -w，不运行来宾代码。
    with tempfile.TemporaryDirectory(prefix='test001-audit-') as tmp:
        snapshot = Path(tmp) / 'image'
        expected = lock['image']
        copy_image_snapshot(image, snapshot, expected['uncompressed_size'])
        with snapshot.open('rb') as stream:
            image_hash = hashlib.sha256()
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                image_hash.update(chunk)
            actual = image_hash.hexdigest()
        if snapshot.stat().st_size != expected['uncompressed_size'] or actual != expected['uncompressed_sha256']:
            raise ValueError('image identity mismatch')
        if shutil.which('debugfs') is None:
            raise subprocess.SubprocessError('debugfs is required for image verification')
        members = {k: v for k, v in lock['files'].items() if v['source'] == 'image'}
        members.update(lock['executables'])
        for name, meta in members.items():
            # 路径来自仓库锁，不接受调用者的 debugfs 命令。
            operation = meta.get('operation', 'cat')
            if operation not in ('cat', 'ls -p'):
                raise ValueError('unsupported locked image operation')
            root_listing = operation == 'ls -p' and meta['path'] == '/'
            if not root_listing and (
                    not re.fullmatch(r'/[a-z0-9_./]+', meta['path']) or '..' in meta['path']):
                raise ValueError('invalid locked image path')
            result = run_debugfs(['-R', operation + ' ' + meta['path'], str(snapshot)],
                                 f'member read: {name}', 30)
            # debugfs 某些读取错误仍退出 0，长度和哈希必须同时符合。
            try:
                check_bytes(result.stdout, meta, name)
            except ValueError as error:
                # 整盘已通过身份校验：此时错误属于提取工具，不是调用者输入。
                diagnostic = result.stderr.decode('utf-8', errors='replace').strip()
                raise subprocess.SubprocessError(
                    f'debugfs member verification failed: {name}; {diagnostic}'
                ) from error
        return {'status': 'VERIFIED', 'sha256': actual, 'members': len(members),
                'tool': run_debugfs(['-V'], 'version query', 10).stderr.decode(
                    'utf-8', errors='replace').strip()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, default=INPUTS)
    parser.add_argument('--verify-image', type=Path)
    args = parser.parse_args()
    try:
        data, lock = load_inputs(args.inputs)
        result = audit(data, lock)
        if args.verify_image:
            result['image_verification'] = verify_image(args.verify_image, lock)
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
