#!/usr/bin/env python3
"""从固定提交拉取并校验官方 judge；只写 third_party，不覆盖已有不同内容。"""
import argparse
import atexit
import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

_source_cache = tempfile.TemporaryDirectory(prefix='focos-source-cache-')
atexit.register(_source_cache.cleanup)
sys.pycache_prefix = _source_cache.name
sys.dont_write_bytecode = True

from inventory import ROOT, digest, load_inventory, validate_official_directory


def fetch(dest: Path) -> None:
    inventory = load_inventory()
    pin = inventory['judge']
    hashes = pin['files']
    if dest.exists():
        judge = dest / 'judge'
        validate_official_directory(judge, hashes)
        if all((judge / name).is_file() and digest(judge / name) == sha for name, sha in hashes.items()):
            print(f'official judges verified: {pin["commit"]}')
            return
        raise ValueError(f'{dest} contains missing/modified judges; use a new --dest, do not overwrite evidence')
    repo = pin['repo'].removeprefix('https://github.com/')
    archive = subprocess.check_output(['gh', 'api', f'repos/{repo}/tarball/{pin["commit"]}'], timeout=60)
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.judges-', dir=dest.parent) as temporary:
        stage = Path(temporary) / 'payload'
        (stage/'judge').mkdir(parents=True)
        with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as source:
            for name in hashes:
                members = [m for m in source.getmembers() if m.isfile() and
                           len(Path(m.name).parts) == 3 and Path(m.name).parts[1:] == ('judge', name)]
                if len(members) != 1:
                    raise ValueError(f'missing/duplicate official judge: {name}')
                with source.extractfile(members[0]) as handle:
                    (stage/'judge'/name).write_bytes(handle.read())
                if digest(stage/'judge'/name) != hashes[name]:
                    raise ValueError(f'official judge hash mismatch: {name}')
        stage.rename(dest)
    print(f'official judges ready: {pin["commit"]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dest', type=Path, default=ROOT/'third_party/oskernel-testsuits-cooperation')
    args = parser.parse_args()
    fetch(args.dest)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError, tarfile.TarError) as exc:
        raise SystemExit(f'fetch judges: {exc}')
