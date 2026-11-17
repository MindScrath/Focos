"""TEST-001 共用输入校验；冻结事实只从 inventory.json 读取。"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / 'tests/inventory.json'
REFERENCE_LOCK = ROOT / 'tests/reference.lock.json'
NAME = re.compile(r'[A-Za-z0-9_][A-Za-z0-9_.+-]*\Z')


def digest(path: Path) -> str:
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def load_inventory(path: Path = INVENTORY, *, raw: bytes | None = None) -> dict:
    data = json.loads(path.read_bytes() if raw is None else raw)
    if not isinstance(data, dict) or type(data.get('schema_version')) is not int or data.get('schema_version') != 1:
        raise ValueError('unsupported inventory schema')
    groups = data.get('groups')
    if not isinstance(groups, dict) or not groups:
        raise ValueError('inventory must contain groups')
    total = 0
    for name, cfg in groups.items():
        if not NAME.fullmatch(name) or not isinstance(cfg, dict):
            raise ValueError(f'invalid group: {name}')
        cases = cfg.get('subcases')
        if (not isinstance(cases, list) or not cases or
                any(not isinstance(n, str) or not n.strip() for n in cases) or
                len(set(cases)) != len(cases)):
            raise ValueError(f'invalid or duplicate subcases: {name}')
        if cfg.get('kind') not in ('official', 'team'):
            raise ValueError(f'invalid judge source: {name}')
        if cfg.get('judge_file') != (f'judge_{name}.py' if cfg['kind'] == 'official'
                                     else f'judge_team/judge_{name}.py'):
            raise ValueError(f'invalid judge path: {name}')
        if cfg.get('mode', 'pass_fail') not in ('pass_fail', 'performance'):
            raise ValueError(f'invalid judge mode: {name}')
        total += len(cases)
    if type(data.get('total_subcases')) is not int or data['total_subcases'] != total:
        raise ValueError('inventory total_subcases mismatch')
    for key in ('source', 'judge'):
        cfg = data.get(key, {})
        if not isinstance(cfg, dict) or not isinstance(cfg.get('commit'), str) or not re.fullmatch(r'[0-9a-f]{40}', cfg['commit']):
            raise ValueError(f'invalid {key} commit')
        if not isinstance(cfg.get('repo'), str) or not re.fullmatch(r'https://github.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', cfg['repo']):
            raise ValueError(f'invalid {key} repository')
    files = data['judge'].get('files', {})
    expected = {g['judge_file'] for g in groups.values() if g['kind'] == 'official'}
    if not isinstance(files, dict) or set(files) != expected or any(not isinstance(v, str) or not re.fullmatch(r'[0-9a-f]{64}', v) for v in files.values()):
        raise ValueError('missing or invalid official judge file hashes')
    image = data.get('image', {})
    if not isinstance(image, dict) or any(not isinstance(image.get(k), str) or not re.fullmatch(r'[0-9a-f]{64}', image[k]) for k in ('published_compressed_sha256', 'uncompressed_sha256')):
        raise ValueError('invalid frozen image hashes')
    return data


def validate_official_directory(directory, names):
    """官方脚本不声明本地依赖；拒绝能遮蔽标准库的额外目录/文件。"""
    if not directory.exists():
        return
    for path in directory.iterdir():
        if path.name == '__pycache__' and path.is_dir() and not path.is_symlink():
            continue
        if path.name not in names or not path.is_file() or path.is_symlink():
            raise ValueError('unfixed official judge dependency: '+str(path))
