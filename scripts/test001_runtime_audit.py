"""TEST-001 A0：脚本启动路径与基盘目录审计，不执行来宾。"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

import test001_audit as common
import test001_ltp_audit as ltp

LOCK = common.ROOT / 'tests/test001/runtime-audit-inputs.lock.json'
INPUTS = common.ROOT / 'tests/test001/runtime-audit-inputs'


def probe_path(path, layout):
    parts = path.split('/')[1:]
    if (not path.startswith('/') or not parts or
            any(p in ('', '.', '..') or not re.fullmatch(r'[A-Za-z0-9_.+-]+', p) for p in parts)):
        raise ValueError('path must be absolute and canonical')
    parent = '/'
    for index, component in enumerate(parts):
        if parent not in layout:
            return {'status': 'NOT_AUDITED', 'uninspected_directory': parent}
        entry = layout[parent].get(component)
        if entry is None:
            return {'status': 'ABSENT_FROM_BASE_IMAGE', 'parent': parent, 'missing_component': component}
        if index == len(parts) - 1:
            return {'status': 'PRESENT_IN_BASE_IMAGE', 'parent': parent, 'entry': entry}
        if entry['kind'] != 'directory':
            return {'status': 'ABSENT_FROM_BASE_IMAGE', 'parent': parent, 'non_directory_component': component}
        parent = parent.rstrip('/') + '/' + component
    raise AssertionError('unreachable path traversal')


def audit():
    directories, lock = common.load_inputs(INPUTS, LOCK)
    scripts, script_lock = common.load_inputs(common.INPUTS)
    ltp_inputs, ltp_lock = common.load_inputs(ltp.INPUTS, ltp.LOCK)
    if not (lock['image'] == script_lock['image'] == ltp_lock['image']):
        raise ValueError('input locks refer to different images')
    layout = {path: {row['name']: row for row in ltp.parse_directory(directories[label + '.debugfs'])}
              for label, path in (('root', '/'), ('musl', '/musl'), ('glibc', '/glibc'))}
    rows, workspaces = [], {}
    for libc in ('musl', 'glibc'):
        for name in ('busybox_testcode.sh', 'lua_testcode.sh', 'test.sh', 'ltp_testcode.sh'):
            source_file = f'{libc}/{name}'
            source_input = f'{libc}-{name}' if name == 'ltp_testcode.sh' else source_file
            data = ltp_inputs if name == 'ltp_testcode.sh' else scripts
            source_lock = ltp_lock if name == 'ltp_testcode.sh' else script_lock
            script_path = probe_path('/' + source_file, layout)
            if (script_path['status'] != 'PRESENT_IN_BASE_IMAGE' or
                    script_path['entry']['kind'] != 'regular' or
                    script_path['entry']['size'] != source_lock['files'][source_input]['size']):
                raise ValueError(f'script directory metadata mismatch: {source_file}')
            first_line = data[source_input].decode('utf-8').split('\n')[0]
            interpreter, argument = None, None
            if first_line.startswith('#!'):
                match = re.fullmatch(r'#!(/[A-Za-z0-9_./-]+)(?: ([A-Za-z0-9_-]+))?', first_line)
                if not match:
                    raise ValueError(f'unsupported shebang: {source_file}')
                interpreter, argument = match.groups()
            probe = probe_path(interpreter, layout) if interpreter else {'status': 'NO_SHEBANG'}
            rows.append({'source_file': source_file, 'source_input': source_input,
                         'source_identity': source_lock['files'][source_input],
                         'line': 1, 'first_line': first_line,
                         'script_path': script_path,
                         'interpreter': interpreter, 'interpreter_argument': argument,
                         'interpreter_status': probe['status'], 'path_evidence': probe})
        workspaces[libc] = {
            'cwd': '/' + libc,
            'programs': {name: probe_path(f'/{libc}/{name}', layout) for name in ('busybox', 'lua', 'test.sh')},
            'write_targets': {name: probe_path(f'/{libc}/{name}', layout)
                              for name in ('test.txt', 'test_dir', 'test', 'busybox_cmd.bak')},
        }
    return {'schema_version': 1, 'status': 'AUDIT_CANDIDATE', 'runtime_ready': False,
            'image_verification': 'NOT_RUN', 'input_lock': lock,
            'scripts': rows, 'workspaces': workspaces,
            'pending': ['launcher and interpreter provisioning', 'PATH and BusyBox applet behavior',
                        'environment, data lifecycle, resources, cleanup and real samples']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify-image', type=Path)
    args = parser.parse_args()
    try:
        result = audit()
        if args.verify_image:
            result['image_verification'] = common.verify_image(args.verify_image, result['input_lock'])
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
