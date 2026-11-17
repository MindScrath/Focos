#!/usr/bin/env python3
"""校验不可变历史证据并用当前判据回放；不覆写原摘要，不把旧环境标为新协议通过。"""
import argparse
import atexit
import copy
import hashlib
import io
import json
from pathlib import Path, PurePosixPath, PureWindowsPath
import subprocess
import sys
import tarfile
import tempfile

_source_cache = tempfile.TemporaryDirectory(prefix='focos-source-cache-')
atexit.register(_source_cache.cleanup)
sys.pycache_prefix = _source_cache.name
sys.dont_write_bytecode = True

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from inventory import NAME, ROOT, REFERENCE_LOCK, INVENTORY, digest

REPLAY_UPDATES = ROOT/'tests/reference-replay-updates.json'


def load_updates(raw,archive_sha256):
    data=json.loads(raw)
    if (not isinstance(data,dict) or type(data.get('schema_version')) is not int or data['schema_version'] != 1 or
            data.get('archive_sha256') != archive_sha256 or not isinstance(data.get('updates'),list)):
        raise ValueError('invalid replay diagnostic updates')
    seen=set()
    for item in data['updates']:
        if (not isinstance(item,dict) or set(item) != {'sample','group','field','before','after','reason'} or
                any(not isinstance(item.get(k),str) or not NAME.fullmatch(item[k]) for k in ('sample','group')) or
                item['field'] != 'unexpected_in_log' or not isinstance(item['reason'],str) or not item['reason'].strip()):
            raise ValueError('replay updates may only change reviewed unexpected_in_log annotations')
        for key in ('before','after'):
            values=item[key]
            if not isinstance(values,list) or any(not isinstance(v,str) or not v for v in values) or sorted(set(values)) != values:
                raise ValueError('invalid replay diagnostic name list')
        key=(item['sample'],item['group'])
        if key in seen:
            raise ValueError('duplicate replay diagnostic update')
        seen.add(key)
    return data['updates']


def expected_groups(sample,original,updates):
    groups=copy.deepcopy(original)
    for item in updates:
        if item['sample'] != sample:
            continue
        targets=[group for group in groups if group['group']==item['group']]
        if len(targets) != 1 or targets[0].get(item['field']) != item['before']:
            raise ValueError('historical diagnostic update does not match original evidence')
        targets[0][item['field']]=list(item['after'])
    return groups


def reject_symlinks(path):
    for component in (path.absolute(),*path.absolute().parents):
        if component.is_symlink():
            raise ValueError('evidence output contains a symlink: '+str(component))


def tree_hashes(directory):
    entries = list(directory.rglob('*'))
    if any(p.is_symlink() or not (p.is_file() or p.is_dir()) for p in entries):
        raise ValueError('evidence tree contains a symlink or special file')
    return {str(p.relative_to(directory)):digest(p) for p in entries if p.is_file()}


def verify_checksums(root):
    checked = set()
    for line in (root/'SHA256SUMS').read_text().splitlines():
        sha, name = line.split(maxsplit=1)
        path = PurePosixPath(name)
        if (len(sha) != 64 or any(c not in '0123456789abcdef' for c in sha) or
                path.is_absolute() or '..' in path.parts or not path.parts or
                str(path) != name or name in checked or name == 'SHA256SUMS'):
            raise ValueError('invalid checksum entry')
        if digest(root/name) != sha:
            raise ValueError('evidence file SHA256 mismatch: ' + name)
        checked.add(name)
    files = set(tree_hashes(root)) - {'SHA256SUMS'}
    if checked != files or len(checked) != 38:
        raise ValueError('incomplete evidence checksum manifest')


def unpack(archive, destination, expected_hash):
    # 用同一字节快照校验并解析，不在复用的目标树上打开任何文件写入。
    reject_symlinks(destination)
    if destination.exists() and not destination.is_dir():
        raise ValueError('evidence destination is not a directory')
    if destination.exists():
        tree_hashes(destination)  # 读取清单前先拒绝所有链接（含断链）。
    with archive.open('rb') as handle:
        raw = handle.read(20_000_001)
    if len(raw) > 20_000_000 or hashlib.sha256(raw).hexdigest() != expected_hash:
        raise ValueError('evidence archive SHA256 mismatch')
    with tarfile.open(fileobj=io.BytesIO(raw),mode='r:gz') as source:
        members = source.getmembers()
        paths = [PurePosixPath(m.name) for m in members]
        if (len({str(p) for p in paths}) != len(paths) or
                any(p.is_absolute() or '..' in p.parts or not p.parts or
                    p.parts[0] != 'test-001-evidence-2026-11-08' for p in paths) or
                any(not (m.isfile() or m.isdir()) for m in members) or
                sum(m.size for m in members) > 20_000_000):
            raise ValueError('invalid evidence archive layout')
        destination.parent.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.evidence-',dir=destination.parent) as temporary:
            stage = Path(temporary)/'payload'
            stage.mkdir()
            for member,path in zip(members,paths):
                target = stage/str(path)
                if member.isdir():
                    target.mkdir(parents=True,exist_ok=True)
                else:
                    target.parent.mkdir(parents=True,exist_ok=True)
                    with source.extractfile(member) as handle,target.open('xb') as output:
                        output.write(handle.read())
            verify_checksums(stage/'test-001-evidence-2026-11-08')
            reject_symlinks(destination)
            if destination.exists() and any(destination.iterdir()):
                if tree_hashes(destination) != tree_hashes(stage):
                    raise ValueError('existing evidence differs; use a new --out, do not overwrite evidence')
            else:
                if destination.exists():
                    destination.rmdir()
                stage.rename(destination)
    return destination/'test-001-evidence-2026-11-08'


def execution_sources(judge_dir):
    paths = {Path(__file__).resolve(), REFERENCE_LOCK, INVENTORY, REPLAY_UPDATES}
    paths.update((ROOT/'scripts').glob('*.py'))
    for directory in (ROOT/'scripts/judge_team', judge_dir):
        paths.update(p for p in directory.rglob('*') if p.is_file() and
                     '__pycache__' not in p.parts and p.suffix not in ('.pyc','.pyo'))
    return paths


def verify_sources(judge_dir, watched):
    if execution_sources(judge_dir) != set(watched) or any(
            not path.is_file() or digest(path) != sha for path,sha in watched.items()):
        raise ValueError('replay source/config changed during run')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--out', type=Path, default=ROOT/'target/test001/replay')
    parser.add_argument('--judge-dir', type=Path, default=ROOT/'third_party/oskernel-testsuits-cooperation/judge')
    args = parser.parse_args(argv)
    watched={path:digest(path) for path in execution_sources(args.judge_dir)}
    lock = json.loads(REFERENCE_LOCK.read_text())['evidence']
    updates=load_updates(REPLAY_UPDATES.read_bytes(),lock['sha256'])
    def unchanged():
        verify_sources(args.judge_dir, watched)
    reject_symlinks(args.out)
    reject_symlinks(args.out/'replay.json')
    args.out.mkdir(parents=True, exist_ok=True)
    archive = args.archive or args.out/lock['asset']
    protected=[archive,*watched]
    if any(path.resolve().is_relative_to((args.out/'current').resolve()) for path in protected):
        raise ValueError('replay input is inside managed current output')
    if not args.archive:
        reject_symlinks(archive)
    report_path = args.out/'replay.json'
    if report_path.exists() and archive.exists() and report_path.samefile(archive):
        raise ValueError('replay report aliases evidence archive')
    if not archive.exists():
        if args.archive:
            raise ValueError('explicit evidence archive missing')
        subprocess.run(['gh', 'release', 'download', lock['tag'], '-R', lock['repo'],
                        '-p', lock['asset'], '-D', str(args.out)], check=True, timeout=60)
    root = unpack(archive, args.out/'original', lock['sha256'])
    samples = sorted((root/'results').glob('*/summary.json'))
    if len(samples) != 15 or len(list((root/'logs').glob('*.log'))) != 7:
        raise ValueError('unexpected number of historical logs/results')
    rows = []
    seen_updates=set()
    for path in samples:
        unchanged()
        original = json.loads(path.read_text())
        log = root/'logs'/PureWindowsPath(original['meta']['log']).name
        out = args.out/'current'/path.parent.name
        reject_symlinks(out)
        for name in ('serial.log','summary.json','summary.md'):
            output=out/name
            if output.exists() and any(p.exists() and output.samefile(p) for p in protected):
                raise ValueError('current replay output aliases an input')
        summary_path=out/'summary.json'
        reject_symlinks(summary_path)
        if summary_path.exists():
            if not summary_path.is_file():
                raise ValueError('current replay summary is not a regular file')
            summary_path.unlink()  # 退出 1 但没有新摘要时，不能借用前次结果。
        cmd = [sys.executable, '-B', str(ROOT/'scripts/run_tests.py'), '--log', str(log),
               '--out', str(out), '--reference', 'legacy', '--libc', original['meta']['libc'],
               '--inventory', str(ROOT/'tests/inventory.json'), '--judge-dir', str(args.judge_dir),
               '--team-judge-dir', str(ROOT/'scripts/judge_team')]
        for group in original['groups']:
            cmd += ['--group', group['group']]
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        if proc.returncode not in (0, 1) or not summary_path.is_file():
            raise ValueError(f'replay failed: {path.parent.name}: {proc.stderr}')
        unchanged()
        current = json.loads(summary_path.read_text())
        # 子进程报告的实际源码/判据也必须属于整轮启动前的版本。
        for field in ('source_sha256', 'judge_sha256'):
            for name, sha in current['meta'].get(field, {}).items():
                source = Path(name)
                source = source if source.is_absolute() else ROOT/source
                expected = next((value for path,value in watched.items() if path.absolute()==source.absolute()), None)
                if expected != sha:
                    raise ValueError('child replay used a different source version: '+name)
        expected_totals = {k: v for k, v in original['totals'].items() if k != 'score'}
        expected=expected_groups(path.parent.name,original['groups'],updates)
        applied=[item for item in updates if item['sample']==path.parent.name]
        seen_updates.update((item['sample'],item['group']) for item in applied)
        row = {'sample': path.parent.name, 'log_sha256': digest(log),
               'groups_match': current['groups'] == original['groups'],
               'expected_groups_match': current['groups'] == expected,
               'diagnostic_updates': applied,
               'totals_match': current['totals'] == expected_totals,
               'environment': current['meta']['environment']['status']}
        rows.append(row)
    if seen_updates != {(item['sample'],item['group']) for item in updates}:
        raise ValueError('unused replay diagnostic updates')
    unchanged()
    report = {'archive_sha256': digest(archive), 'historical_runner_commit': lock['historical_runner_commit'],
              'runner_sha256': watched[ROOT/'scripts/run_tests.py'],
              'source_sha256': {str(path):sha for path,sha in watched.items()}, 'samples': rows,
              'all_match': all(r['groups_match'] and r['totals_match'] for r in rows),
              'all_expected_match': all(r['expected_groups_match'] and r['totals_match'] for r in rows)}
    if digest(archive) != lock['sha256']:
        raise ValueError('evidence archive changed during replay')
    # 原子替换断开已有硬链接，不能通过报告写入改动别处的文件。
    with tempfile.TemporaryDirectory(prefix='.report-',dir=args.out) as temporary:
        stage = Path(temporary)/'replay.json'
        stage.write_text(json.dumps(report,indent=2)+'\n')
        reject_symlinks(report_path)
        if report_path.exists() and report_path.samefile(archive):
            raise ValueError('replay report aliases evidence archive')
        stage.replace(report_path)
    for row in rows:
        print(f"{row['sample']}: groups={row['groups_match']} expected={row['expected_groups_match']} totals={row['totals_match']} diagnostics={len(row['diagnostic_updates'])} ({row['environment']})")
    return 0 if report['all_expected_match'] else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, OSError, subprocess.SubprocessError, tarfile.TarError) as exc:
        raise SystemExit(f'replay evidence: {exc}')
