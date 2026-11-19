"""TEST-001 A0：检查四个已锁定 ELF，输出 readelf 原始证据，不执行来宾。"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import test001_audit as common


def run_readelf(arguments):
    try:
        result = subprocess.run(['readelf', *arguments], capture_output=True,
                                check=True, timeout=30, env={**os.environ, 'LC_ALL': 'C'})
    except (OSError, subprocess.SubprocessError) as error:
        diagnostic = getattr(error, 'stderr', None) or b''
        raise subprocess.SubprocessError(f'readelf failed: {error}; {diagnostic!r}') from error
    if result.stderr or not result.stdout:
        raise subprocess.SubprocessError(f'readelf incomplete or diagnostic output: {result.stderr!r}')
    try:
        return result.stdout.decode('utf-8')
    except UnicodeDecodeError as error:
        raise subprocess.SubprocessError('readelf output is not UTF-8') from error


def inspect_binaries(root):
    lock = json.loads(common.LOCK.read_bytes())
    with tempfile.TemporaryDirectory(prefix='test001-elf-') as tmp:
        copies = {}
        # 全部先验证，绝不把未验证二进制交给工具。
        for name, meta in lock['executables'].items():
            filename = name.replace('/', '-')
            private = Path(tmp) / filename
            common.copy_image_snapshot(Path(root) / filename, private, meta['size'])
            common.check_bytes(private.read_bytes(), meta, name)
            copies[name] = private
        version = run_readelf(['--version'])
        entries = {}
        for name, private in copies.items():
            entries[name] = {**lock['executables'][name],
                             'readelf_stdout': run_readelf(['-h', '-l', '-d', '-W', str(private)])}
        return {'schema_version': 1, 'status': 'AUDIT_EVIDENCE', 'runtime_ready': False,
                'image_verification': 'NOT_RUN', 'tool_version': version,
                'tool_arguments': ['-h', '-l', '-d', '-W'], 'tool_locale': 'C',
                'executables': entries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binaries', required=True, type=Path)
    args = parser.parse_args()
    try:
        report = inspect_binaries(args.binaries)
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
        return 0
    except (ValueError, FileNotFoundError) as error:
        print(f'audit input error: {error}', file=sys.stderr)
        return 2
    except (OSError, subprocess.SubprocessError) as error:
        print(f'audit execution error: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
