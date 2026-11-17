#!/usr/bin/env python3
"""TEST-001 test runner.

Capture or replay a serial log, split it into OS COMP test groups, run the
official (or team) judge for each group, and emit a per-subcase result table.

Design notes:
- The runner never decides pass/fail by itself. It only normalizes the judge
  output and marks missing/truncated/error cases as not passed.
- Missing group -> NOT_RUN (all subcases 0). START without END -> TIMEOUT.
- Judge crash -> JUDGE_ERROR (all subcases 0). Unknown judge names -> extras,
  excluded from the inventory denominator.
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import math
import signal
import hashlib
import shutil
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

# -B 只禁写 pyc；项目导入前切换到本次运行的私有空缓存，避免读取旧代码。
_source_cache = tempfile.TemporaryDirectory(prefix='focos-source-cache-')
atexit.register(_source_cache.cleanup)
sys.pycache_prefix = _source_cache.name
sys.dont_write_bytecode = True

from inventory import load_inventory, digest, ROOT, INVENTORY, REFERENCE_LOCK, validate_official_directory
from reference_protocol import validate_reference
from input_snapshot import InputSnapshot
from lmbench_trace import logged_names as lmbench_logged_names
from reference_manifest import load_manifest, verify_reference_inputs, verify_reference_log, strict_qemu_command, qemu_identity

GROUP_RE = re.compile(r"^####\s*OS COMP TEST GROUP\s+(START|END)\s+(\S+)\s*####\s*$")
LIBC_SUFFIX_RE = re.compile(r"-(musl|glibc)$")


@dataclass
class Section:
    group: str
    libc: str
    lines: list[str] = field(default_factory=list)
    truncated: bool = False

    @property
    def text(self) -> str:
        return "\n".join(self.lines)


@dataclass
class JudgeResult:
    items: list[dict]
    source: str
    error: str | None = None


def normalize_group(raw: str, default_libc: str) -> tuple[str, str]:
    match = LIBC_SUFFIX_RE.search(raw)
    if match:
        return raw[: match.start()], match.group(1)
    return raw, default_libc


def split_sections(log_text: str, default_libc: str) -> dict[tuple[str, str], Section]:
    """严格配对组与 libc；重复、嵌套或孤立 END 不允许被覆盖。"""
    sections = {}
    seen = set()
    current = None
    for line in log_text.splitlines():
        match = GROUP_RE.fullmatch(line)
        if not match:
            if '#### OS COMP TEST GROUP' in line and not line.startswith('+ '):
                raise ValueError('malformed group marker')
            if current is not None:
                current.lines.append(line)
            continue
        state, raw = match.groups()
        group, libc = normalize_group(raw, default_libc)
        key = (group, libc)
        if state == 'START':
            if current is not None or key in seen:
                raise ValueError(f'nested or duplicate section: {raw}')
            seen.add(key)
            current = Section(group=group, libc=libc)
        else:
            if current is None or (current.group, current.libc) != key:
                raise ValueError(f'unmatched END: {raw}')
            sections[key] = current
            current = None
    if current is not None:
        current.truncated = True
        sections[(current.group, current.libc)] = current
    return sections


def judge_files(directory):
    return {p for p in directory.rglob('*') if p.is_file() and
            '__pycache__' not in p.parts and p.suffix not in ('.pyc', '.pyo')}


def freeze_judge_sources(paths, expected):
    frozen = {}
    for path in paths:
        if not path.is_file():
            continue
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != expected.get(str(path)):
            raise ValueError('judge source changed while freezing: '+str(path))
        frozen[path.absolute()] = raw
    return frozen


def run_judge(judge_path: Path, section_text: str, timeout: int, *, sources=None) -> JudgeResult:
    judge_path = judge_path.absolute()
    if sources is None:
        try:
            paths = judge_files(judge_path.parent) | {ROOT/'scripts/inventory.py', ROOT/'scripts/lmbench_trace.py', INVENTORY, REFERENCE_LOCK}
            sources = {p.absolute():p.read_bytes() for p in paths}
        except OSError as exc:
            return JudgeResult([], 'error', f'judge snapshot failed: {exc}')
    if judge_path not in sources:
        return JudgeResult([], "missing", f"judge not found: {judge_path}")
    if judge_path.suffix == ".sh":
        return JudgeResult([], "unsupported", f"shell judge is not a stdin parser: {judge_path}")
    try:
        with tempfile.TemporaryDirectory(prefix='focos-judge-source-') as private:
            root = Path(private)
            directory = root/'scripts/judge_team'
            payload = {directory/p.relative_to(judge_path.parent):raw for p,raw in sources.items()
                       if p.is_relative_to(judge_path.parent)}
            for relative in ('scripts/inventory.py','scripts/lmbench_trace.py','tests/inventory.json','tests/reference.lock.json'):
                if ROOT/relative in sources:
                    payload[root/relative] = sources[ROOT/relative]
            for path,raw in payload.items():
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(raw)
                path.chmod(0o444)
            env = dict(os.environ)
            env.pop('PYTHONPATH',None)
            proc = subprocess.run(
                [sys.executable, '-B', '-X', 'pycache_prefix='+str(root/'cache'), str(directory/judge_path.name)],
                input=section_text,
                text=True,
                encoding="utf-8",
                errors="replace",
                capture_output=True,
                timeout=timeout,
                env=env,
            )
    except subprocess.TimeoutExpired:
        return JudgeResult([], "timeout", f"judge timed out after {timeout}s")
    except OSError as exc:
        return JudgeResult([], "error", f"judge failed to start: {exc}")

    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()[:2000]
        return JudgeResult([], "error", f"judge exited {proc.returncode}: {detail}")

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON field: ' + key)
            result[key] = value
        return result
    def reject_constant(value):
        raise ValueError('nonfinite JSON constant: ' + value)
    try:
        data = json.loads(proc.stdout, object_pairs_hook=unique_object, parse_constant=reject_constant)
    except ValueError as exc:
        return JudgeResult([], "error", f"judge output is not JSON: {exc}; stdout={proc.stdout[:500]!r}")
    if not isinstance(data, list) or not all(isinstance(x, dict) for x in data):
        return JudgeResult([], "error", "judge output must be a JSON list of objects")
    return JudgeResult(data, "ok")


def normalize_item(item: dict, mode: str) -> tuple[str, int, int, float]:
    name = item.get('name')
    if not isinstance(name, str) or not name.strip() or name != name.strip():
        raise ValueError('invalid judge name')
    score = item.get('score', 0)
    if type(score) not in (int, float) or not math.isfinite(score) or score < 0:
        raise ValueError('invalid/nonfinite judge score')
    if mode == 'performance':
        value = item.get('result')
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ValueError('invalid/nonfinite performance result')
        return name, 1, int(value > 0), float(score)
    if 'all' in item and 'total' in item and item['all'] != item['total']:
        raise ValueError('conflicting all/total assertion counts')
    all_count = item.get('all', item.get('total'))
    passed = item.get('pass')
    if (type(all_count) is not int or type(passed) is not int or
            all_count <= 0 or not 0 <= passed <= all_count):
        raise ValueError('invalid judge assertion counts')
    return name, all_count, passed, float(score)


def judge_group(
    group: str,
    section: Section | None,
    cfg: dict,
    judge_dir: Path,
    team_dir: Path,
    judge_timeout: int,
    official_hashes: dict | None = None,
    judge_sources: dict | None = None,
) -> dict:
    expected = list(cfg.get("subcases", []))
    mode = cfg.get("mode", "pass_fail")
    result = {
        "group": group,
        "libc": section.libc if section else None,
        "status": "NOT_RUN",
        "judge_source": cfg.get("kind", "unknown"),
        "judge_error": None,
        "subcases": [],
        "extras": [],
        "unexpected_in_log": [],
        "section_found": section is not None,
        "section_truncated": bool(section and section.truncated),
    }
    if section is not None:
        logged = {
            name.strip()
            for name in re.findall(r"^=+\s*START\s+(.+?)\s*=+\s*$", section.text, re.M)
        }
        if group in ('iperf', 'netperf', 'cyclictest'):
            logged.update(re.findall(rf'^=+\s*{group}\s+(\S+)\s+end:\s*\S+\s*=+\s*$', section.text, re.M))
        elif group == 'unixbench':
            logged.update('Unixbench '+name.strip() for name in re.findall(r'^Unixbench (.+?):', section.text, re.M))
        elif group == 'libcbench':
            logged.update(re.findall(r'^b_[^\r\n(]+\([^\r\n]*\)\s*$', section.text, re.M))
            logged = {name.strip() for name in logged}
        elif group == 'lua':
            logged.update('lua '+name for name in re.findall(r'^testcase[ \t]+lua[ \t]+(\S+)[ \t]+\S+[ \t]*$',section.text,re.M))
        elif group == 'busybox':
            logged.update(name.strip() for name in re.findall(r'^testcase[ \t]+(.+?)[ \t]+\S+[ \t]*$',section.text,re.M))
        elif group == 'lmbench':
            logged.update(lmbench_logged_names(section.text,expected))
        if group == "libctest":
            normalized = set()
            for name in logged:
                if name.startswith("entry-static.exe "):
                    normalized.add("libctest static " + name[len("entry-static.exe "):])
                elif name.startswith("entry-dynamic.exe "):
                    normalized.add("libctest dynamic " + name[len("entry-dynamic.exe "):])
                else:
                    normalized.add(name)
            logged = normalized
        result["unexpected_in_log"] = sorted(logged - set(expected))

    if section is None:
        result["subcases"] = [
            {"name": n, "status": "NOT_RUN", "all": 1, "pass": 0, "score": 0.0}
            for n in expected
        ]
        return result

    judge_file = cfg.get("judge_file", "")
    if cfg.get("kind") == "official":
        judge_path = judge_dir / Path(judge_file).name
    else:
        judge_path = team_dir / Path(judge_file).name

    if cfg.get('kind') == 'official' and official_hashes is not None:
        # 官方脚本没有本地依赖；只将固定集合交给子进程，关闭目录检查后的新增窗口。
        if judge_sources is None:
            try:
                validate_official_directory(judge_dir, official_hashes)
                judge_sources = {p.absolute():p.read_bytes() for p in
                                 (judge_dir/name for name in official_hashes) if p.is_file()}
            except (ValueError,OSError) as exc:
                judge_sources = {}
        else:
            judge_sources = {p:raw for p,raw in judge_sources.items()
                             if p.parent == judge_path.absolute().parent and p.name in official_hashes}
    frozen = judge_sources.get(judge_path.absolute()) if judge_sources is not None else None
    actual_hash = hashlib.sha256(frozen).hexdigest() if frozen is not None else (digest(judge_path) if judge_path.is_file() else None)
    if cfg.get('kind') == 'official' and official_hashes is not None and actual_hash is not None and actual_hash != official_hashes.get(judge_path.name):
        judged = JudgeResult([], 'error', 'official judge hash mismatch')
    else:
        judged = run_judge(judge_path, section.text, judge_timeout, sources=judge_sources) if judge_sources is not None else run_judge(judge_path, section.text, judge_timeout)
    result["judge_source"] = cfg.get("kind", "unknown")
    def judge_error(message):
        result['status'] = 'TIMEOUT' if section.truncated else 'JUDGE_ERROR'
        result['judge_error'] = message
        result['subcases'] = [
            {'name': name, 'status': result['status'], 'all': 1, 'pass': 0, 'score': 0.0}
            for name in expected]
        return result
    if judged.error:
        return judge_error(judged.error)

    by_name = {}
    try:
        for item in judged.items:
            name, all_count, passed, score = normalize_item(item, mode)
            if name in by_name:
                raise ValueError('duplicate judge name: ' + name)
            by_name[name] = {'all': all_count, 'pass': passed, 'score': score}
    except (ValueError, TypeError, OverflowError) as exc:
        return judge_error(str(exc))

    subcases = []
    for name in expected:
        item = by_name.pop(name, None)
        if item is None:
            subcases.append({"name": name, "status": "MISSING", "all": 1, "pass": 0, "score": 0.0})
            continue
        if item["pass"] == item["all"] and item["all"] > 0:
            status = "PASS"
        elif item["pass"] > 0:
            status = "PARTIAL"
        else:
            status = "FAIL"
        subcases.append({"name": name, "status": status, **item})

    result["subcases"] = subcases
    result["extras"] = sorted(by_name.keys())

    if section.truncated:
        result["status"] = "TIMEOUT"
    elif all(x["status"] == "PASS" for x in subcases):
        result["status"] = "PASS"
    elif any(x["status"] in ("PASS", "PARTIAL") for x in subcases):
        result["status"] = "PARTIAL"
    else:
        result["status"] = "FAIL"
    return result


def execution_status(result, *, environment_error, capture_timeout, log_error, capture_exit, script_exit,
                     reference_incomplete=False):
    """所有执行原因在一处归并；段级截断不能被较低优先级的退出码覆盖。"""
    priorities = (
        (environment_error, 'ENV_ERROR'),
        (capture_timeout or result['section_truncated'], 'TIMEOUT'),
        (reference_incomplete, 'ENV_ERROR'),
        (bool(log_error), 'LOG_ERROR'),
        (capture_exit not in (None,0), 'CAPTURE_ERROR'),
        (script_exit != 0 and result['status'] in ('PASS','PARTIAL','FAIL'), 'FAIL'),
    )
    return next((status for applies,status in priorities if applies),result['status'])


def capture(cmd: str | list[str], timeout: int) -> tuple[bytes, bool, int | None]:
    """超时关闭整个进程组，避免 QEMU/后代继续持有串口管道。"""
    proc = subprocess.Popen(cmd, shell=isinstance(cmd, str), stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, start_new_session=True)
    truncated = False
    try:
        out, _ = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        truncated = True
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        out, _ = proc.communicate()
    return out, truncated, proc.returncode


def summarize(results: list[dict]) -> dict:
    totals = {
        "subcases": 0,
        "pass": 0,
        "partial": 0,
        "fail": 0,
        "not_run": 0,
        "groups": {"PASS": 0, "PARTIAL": 0, "FAIL": 0, "TIMEOUT": 0, "NOT_RUN": 0, "JUDGE_ERROR": 0},
    }
    for result in results:
        totals["groups"][result["status"]] = totals["groups"].get(result["status"], 0) + 1
        for sub in result["subcases"]:
            totals["subcases"] += 1
            status = sub["status"]
            if status == "PASS":
                totals["pass"] += 1
            elif status == "PARTIAL":
                totals["partial"] += 1
            elif status == "NOT_RUN":
                totals["not_run"] += 1
            else:
                totals["fail"] += 1
    totals["pass_rate"] = round(totals["pass"] / totals["subcases"], 4) if totals["subcases"] else 0.0
    return totals


def reject_output_links(path):
    for component in (path.absolute(), *path.absolute().parents):
        if component.is_symlink():
            raise ValueError('output path contains a symlink: '+str(component))


def atomic_write(path, raw):
    reject_output_links(path.parent)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.result-', delete=False) as handle:
        stage = Path(handle.name)
        try:
            handle.write(raw)
            handle.close()
            stage.replace(path)
        finally:
            stage.unlink(missing_ok=True)


def write_summary(out_dir: Path, meta: dict, results: list[dict], totals: dict) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    payload = {"meta": meta, "totals": totals, "groups": results}
    atomic_write(out_dir / "summary.json", json.dumps(payload, ensure_ascii=False, indent=2, allow_nan=False).encode("utf-8"))

    lines = [
        "# TEST-001 result summary",
        "",
        f"- platform: {meta.get('platform')}",
        f"- libc: {meta.get('libc')}",
        f"- inventory: {meta.get('inventory')}",
        f"- log: {meta.get('log')}",
        f"- mode: {meta['mode']}",
        f"- {'captured_at' if meta['mode'] == 'capture' else 'replayed_at'}: {meta.get('captured_at', meta.get('replayed_at'))}",
        f"- environment: {meta['environment']['status']}",
        f"- execution_valid: {meta['execution_valid']}",
        f"- errors: {meta['environment']['errors'] or meta['log_error'] or '-'}",
        "",
        "| group | libc | status | judge | subcases | pass | partial | fail | not_run |",
        "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for result in results:
        counts = {"PASS": 0, "PARTIAL": 0, "FAIL": 0, "NOT_RUN": 0}
        for sub in result["subcases"]:
            key = sub["status"] if sub["status"] in counts else "FAIL"
            counts[key] += 1
        lines.append(
            f"| {result['group']} | {result['libc'] or '-'} | {result['status']} | {result['judge_source']} | "
            f"{len(result['subcases'])} | {counts['PASS']} | {counts['PARTIAL']} | {counts['FAIL']} | {counts['NOT_RUN']} |"
        )
    lines += [
        "",
        f"**Total**: {totals['subcases']} subcases, {totals['pass']} pass, {totals['partial']} partial, "
        f"{totals['fail']} fail, {totals['not_run']} not run; pass rate {totals['pass_rate']:.2%}",
        "",
        "A group is PASS only when every inventory subcase passes. TIMEOUT/NOT_RUN/JUDGE_ERROR are not passes.",
        "Scores remain in per-case JSON only; mixed units are never summed.",
    ]
    atomic_write(out_dir / "summary.md", "\n".join(lines).encode("utf-8"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="TEST-001 runner")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--log", help="replay an existing serial log")
    source.add_argument("--capture-cmd", help="run a command (e.g. QEMU) and capture stdout")
    parser.add_argument("--out", required=True, help="output directory")
    parser.add_argument("--group", action="append", default=None, help="group to judge; repeatable; default all")
    parser.add_argument("--libc", required=True, choices=["musl", "glibc"])
    parser.add_argument("--reference", choices=["strict", "legacy"], help="严格新协议或显式回放未验证的旧环境")
    parser.add_argument('--capture-artifact', action='append', type=Path, default=[], help='被测内核/镜像等文件，可重复；普通 capture 必填')
    parser.add_argument('--reference-image', type=Path, help='严格参考环境实际使用的磁盘镜像')
    parser.add_argument('--reference-manifest', type=Path)
    parser.add_argument('--reference-artifacts', nargs=2, metavar=('KERNEL', 'INITRAMFS'))
    parser.add_argument("--timeout", type=int, default=600, help="capture timeout in seconds")
    parser.add_argument("--judge-timeout", type=int, default=120, help="judge timeout in seconds")
    parser.add_argument("--judge-dir", default="third_party/oskernel-testsuits-cooperation/judge")
    parser.add_argument("--team-judge-dir", default="scripts/judge_team")
    parser.add_argument("--inventory", default="tests/inventory.json")
    parser.add_argument("--platform", default="qemu")
    args = parser.parse_args(argv)
    if (args.capture_cmd is not None and not args.capture_cmd.strip()) or args.log == '':
        print('capture command or log source must not be empty',file=sys.stderr)
        return 2
    if args.reference == 'strict' and (args.reference_manifest is None or args.reference_artifacts is None or args.reference_image is None):
        print('strict requires --reference-manifest, --reference-artifacts KERNEL INITRAMFS and --reference-image',file=sys.stderr)
        return 2
    if args.reference_manifest is not None:
        if args.reference == 'legacy':
            print('reference manifest cannot be combined with legacy',file=sys.stderr)
            return 2
        args.reference = 'strict'
    if args.reference == 'strict' and args.platform != 'qemu':
        print('strict reference requires --platform qemu',file=sys.stderr)
        return 2
    snapshot = InputSnapshot(digest)
    # Python 已加载的实现必须先固定身份，不能等耗时的大构件哈希后才读源码版本。
    source_hashes = {str(p.relative_to(ROOT)): snapshot.digest(p) for p in
                    [Path(__file__).resolve(), ROOT/'scripts/input_snapshot.py', ROOT/'scripts/inventory.py',
                     ROOT/'scripts/reference_protocol.py', ROOT/'scripts/reference_manifest.py', ROOT/'scripts/lmbench_trace.py']}

    out_dir = Path(args.out)
    try:
        reject_output_links(out_dir)
    except ValueError as exc:
        print(str(exc),file=sys.stderr)
        return 2
    sections_dir = out_dir/'sections'
    if sections_dir.is_symlink() or (sections_dir.exists() and not sections_dir.is_dir()):
        print('sections must be a real directory', file=sys.stderr)
        return 2
    inputs = ([args.reference_image] if args.reference_image else []) + [Path(args.inventory)] + ([Path(args.log)] if args.log else []) + args.capture_artifact + [Path(p) for p in (args.reference_artifacts or [])] + ([args.reference_manifest] if args.reference_manifest else [])
    if any(p.resolve().is_relative_to(sections_dir.resolve()) for p in inputs):
        print('input files cannot be stored in managed output sections', file=sys.stderr)
        return 2
    if args.capture_cmd and not (args.capture_artifact or args.reference_artifacts):
        print('capture requires --capture-artifact or --reference-artifacts', file=sys.stderr)
        return 2
    artifact_paths = ([args.reference_image] if args.reference_image else []) + args.capture_artifact + [Path(p) for p in (args.reference_artifacts or [])]
    if any(not p.is_file() for p in artifact_paths):
        print('capture artifacts must be existing regular files', file=sys.stderr)
        return 2
    inventory_path = Path(args.inventory)
    if not inventory_path.exists():
        print(f"inventory not found: {inventory_path}", file=sys.stderr)
        return 2
    try:
        inventory_bytes = inventory_path.read_bytes()
        inventory_hash = hashlib.sha256(inventory_bytes).hexdigest()
        inventory = load_inventory(inventory_path, raw=inventory_bytes)
    except (ValueError, OSError, KeyError, TypeError) as exc:
        print(f'invalid inventory: {exc}', file=sys.stderr)
        return 2
    if args.timeout <= 0 or args.judge_timeout <= 0:
        print('timeouts must be positive', file=sys.stderr)
        return 2
    groups_cfg = inventory["groups"]
    selected = args.group or list(groups_cfg.keys())
    if len(set(selected)) != len(selected):
        print("duplicate selected groups", file=sys.stderr)
        return 2
    unknown = [g for g in selected if g not in groups_cfg]
    if unknown:
        print(f"unknown group(s): {', '.join(unknown)}", file=sys.stderr)
        return 2

    canonical = load_inventory()
    if inventory['judge'] != canonical['judge'] or any(
            group not in canonical['groups'] or any(
                cfg.get(key, 'pass_fail' if key == 'mode' else None) !=
                canonical['groups'][group].get(key, 'pass_fail' if key == 'mode' else None)
                for key in ('kind','judge_file','mode'))
            for group,cfg in groups_cfg.items()):
        print('inventory changes frozen judge provenance',file=sys.stderr)
        return 2
    for group in selected:
        cfg = groups_cfg[group]
        if cfg['kind'] == 'team' and (group not in canonical['groups'] or
                cfg['subcases'] != canonical['groups'][group]['subcases'] or
                cfg.get('mode', 'pass_fail') != canonical['groups'][group].get('mode', 'pass_fail')):
            print('team parser does not support this inventory snapshot: '+group, file=sys.stderr)
            return 2

    manifest, manifest_hash = None, None
    if args.reference_manifest:
        try:
            manifest_bytes = args.reference_manifest.read_bytes()
            manifest = load_manifest(args.reference_manifest,raw=manifest_bytes)
            manifest_hash = hashlib.sha256(manifest_bytes).hexdigest()
        except (ValueError,OSError) as exc:
            print(f'invalid reference manifest: {exc}',file=sys.stderr)
            return 2
    try:
        artifact_hashes = {str(p):snapshot.digest(p) for p in artifact_paths}
        preflight_provenance = None
        if manifest is not None:
            preflight_provenance = verify_reference_inputs(manifest,args.reference_artifacts or [],inventory,
                                                          selected,args.libc,inventory_path,args.reference_image,
                                                          file_digest=snapshot.digest)
    except (ValueError,OSError,KeyError,TypeError) as exc:
        print(f'invalid execution inputs: {exc}',file=sys.stderr)
        return 2
    capture_command = args.capture_cmd
    command_verified = False
    qemu = None
    if args.capture_cmd and (args.reference == 'strict' or args.reference_manifest):
        try:
            if manifest is None:
                raise ValueError('strict capture requires --reference-manifest')
            capture_command = strict_qemu_command(args.capture_cmd, args.reference_artifacts or [], args.reference_image)
        except (ValueError, OSError, TypeError) as exc:
            print(f'invalid strict capture: {exc}', file=sys.stderr)
            return 2

    # 在执行外部命令前固定源码身份，长时间运行期间的编辑不能改写这次执行的来源。
    # 所有会影响判分的文件在采集前登记；缺失文件也登记，防止中途补入。
    try:
        validate_official_directory(Path(args.judge_dir), inventory['judge']['files'])
    except ValueError as exc:
        print(str(exc),file=sys.stderr)
        return 2
    judge_paths = judge_files(Path(args.judge_dir)) | judge_files(Path(args.team_judge_dir))
    for group in selected:
        cfg = groups_cfg[group]
        directory = Path(args.judge_dir) if cfg['kind'] == 'official' else Path(args.team_judge_dir)
        judge_paths.add(directory/Path(cfg['judge_file']).name)
    judge_hashes = {str(p): snapshot.digest(p) for p in sorted(judge_paths) if p.is_file()}
    for name,sha in source_hashes.items():
        snapshot.watch(ROOT/name,sha)
    for p in judge_paths:
        snapshot.watch(p,judge_hashes.get(str(p)))
    for p,sha in artifact_hashes.items():
        snapshot.watch(p,sha,bulk=True)
    for p in {inventory_path, INVENTORY, REFERENCE_LOCK}:
        snapshot.watch(p,inventory_hash if p == inventory_path else snapshot.digest(p))
    try:
        execution_paths = judge_paths | {ROOT/'scripts/inventory.py', ROOT/'scripts/lmbench_trace.py', INVENTORY, REFERENCE_LOCK}
        execution_hashes = {str(p):snapshot.digest(p) for p in execution_paths if p.is_file()}
        judge_sources = freeze_judge_sources(execution_paths,execution_hashes)
    except (ValueError,OSError) as exc:
        print(f'invalid judge sources: {exc}',file=sys.stderr)
        return 2
    if args.reference_manifest:
        snapshot.watch(args.reference_manifest,manifest_hash)
    if isinstance(capture_command,list):
        try:
            qemu = qemu_identity(capture_command[0])
            capture_command[0] = qemu['path']
            artifact_hashes[qemu['path']] = qemu['sha256']
            snapshot.watch(qemu['path'],qemu['sha256'],bulk=True)
            snapshot.watch(qemu['invoked_path'],qemu['sha256'],bulk=True)
            command_verified = True
        except (ValueError,OSError) as exc:
            print(f'invalid strict QEMU: {exc}',file=sys.stderr)
            return 2
    watched = snapshot.states
    managed_files = [out_dir/name for name in ('serial.log', 'summary.json', 'summary.md')]
    def aliases(a, b):
        if a.resolve() == b.resolve(): return True
        try: return a.samefile(b)
        except OSError: return False
    def managed_outputs_overlap():
        from itertools import combinations
        return any(aliases(a,b) for a,b in combinations(managed_files,2))
    if managed_outputs_overlap():
        print('managed output paths overlap',file=sys.stderr)
        return 2
    for p in inputs:
        for target in managed_files:
            inplace_log = args.log and p.resolve() == Path(args.log).resolve() == (out_dir/'serial.log').resolve() and target.name == 'serial.log'
            if aliases(p, target) and not inplace_log:
                print('input aliases managed output', file=sys.stderr)
                return 2
    if any(p.is_symlink() for p in managed_files) or any(
            p.resolve().is_relative_to(sections_dir.resolve()) or
            any(aliases(p, target) for target in managed_files) for p in watched):
        print('managed output aliases an execution input or symlink', file=sys.stderr)
        return 2
    if snapshot.changes():
        print('execution inputs changed before capture/replay',file=sys.stderr)
        return 2
    truncated = False
    if args.log:
        raw_log = Path(args.log).read_bytes()
        log_label = str(Path(args.log))
        exit_code = None
    else:
        raw_log, truncated, exit_code = capture(capture_command, args.timeout)
        log_label = args.capture_cmd

    log_text = raw_log.decode('utf-8', errors='replace')
    log_error = None
    try:
        sections = split_sections(log_text, args.libc)
    except ValueError as exc:
        sections, log_error = {}, str(exc)
    environment = validate_reference(log_text, args.reference, selected, args.libc)
    changed = snapshot.changes()
    provenance = None
    strict = args.reference == 'strict' or (args.reference is None and environment['status'] != 'NOT_APPLICABLE')
    if strict and args.platform != 'qemu':
        print('strict reference requires --platform qemu',file=sys.stderr)
        return 2
    if strict and not changed:
        try:
            if args.reference_manifest is None or args.reference_artifacts is None:
                raise ValueError('strict reference requires --reference-manifest and --reference-artifacts KERNEL INITRAMFS')
            if args.capture_cmd and not command_verified:
                raise ValueError('strict capture command was not validated before execution')
            allow_incomplete = environment.get('incomplete',False) and (truncated or any(
                section.truncated for (group,libc),section in sections.items() if group in selected and libc==args.libc))
            verify_reference_log(manifest,log_text,allow_incomplete=allow_incomplete)
            provenance = {'manifest_sha256':manifest_hash,**preflight_provenance}
        except (ValueError, OSError, KeyError, TypeError) as exc:
            environment['status'] = 'ERROR'
            environment['errors'].append(str(exc))
            environment['incomplete'] = False
    results = []
    for group in selected:
        changed = sorted(set(changed + snapshot.changes()))
        section = None if changed else sections.get((group, args.libc))
        result = judge_group(
            group,
            section,
            groups_cfg[group],
            Path(args.judge_dir),
            Path(args.team_judge_dir),
            args.judge_timeout,
            inventory['judge']['files'],
            judge_sources,
        )
        results.append(result)

    changed = sorted(set(changed + snapshot.changes(full=True)))
    if changed:
        environment['status'] = 'ERROR'
        environment['errors'].append('inputs changed during run: '+', '.join(changed))
    reference_incomplete = environment['status']=='ERROR' and environment.get('incomplete',False)
    environment_error = bool(changed or re.search(r'^[ \t]*REF_FAIL(?:\s|$)',log_text,re.M) or
                             (environment['status']=='ERROR' and not reference_incomplete))
    invalid_execution = environment_error or reference_incomplete or truncated or bool(log_error) or exit_code not in (None,0)
    for result in results:
        result['libc'] = args.libc
        script_exit = environment['script_exits'].get(f"{result['group']}-{args.libc}",0)
        script_failure = script_exit != 0 and result['status'] in ('PASS','PARTIAL','FAIL')
        result['status'] = execution_status(result,environment_error=environment_error,capture_timeout=truncated,
                                            log_error=log_error,capture_exit=exit_code,script_exit=script_exit,
                                            reference_incomplete=reference_incomplete)
        if invalid_execution or script_failure or (result['status']=='TIMEOUT' and args.reference!='legacy'):
            for sub in result['subcases']:
                sub['observed'] = {key: sub[key] for key in ('status', 'pass', 'score')}
                sub.update(status=result['status'], **{'pass': 0, 'score': 0.0})

    replay_unverified = bool(args.log and strict)
    if replay_unverified and environment['status'] == 'VALID':
        environment['status'] = 'REPLAY_UNVERIFIED'
        environment['errors'].append('log protocol and artifacts match; original capture argv, QEMU and platform are unverified')
    totals = summarize(results)
    import datetime

    meta = {
        "platform": args.platform,
        "libc": args.libc,
        "inventory": str(inventory_path),
        "log": log_label,
        "capture_exit_code": exit_code,
        "capture_argv": capture_command if isinstance(capture_command, list) else None,
        "capture_truncated": truncated,
        "mode": "capture" if args.capture_cmd else "replay",
        'inventory_sha256': inventory_hash,
        'artifact_sha256': artifact_hashes,
        'log_sha256': hashlib.sha256(raw_log).hexdigest(),
        'environment': environment,
        'reference_provenance': provenance,
        'reference_command_verified': command_verified,
        'qemu': qemu,
        'capture_conditions_verified': bool(args.capture_cmd and command_verified and not invalid_execution),
        'execution_valid': not replay_unverified and not invalid_execution and environment['status'] != 'LEGACY_UNVERIFIED' and not any(
            section.truncated for (group,libc),section in sections.items() if group in selected and libc == args.libc),
        'log_error': log_error,
        'source_sha256': source_hashes,
        'judge_sha256': judge_hashes,
        'changed_inputs': changed,
    }
    timestamp = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds')
    if args.capture_cmd:
        meta['captured_at'] = timestamp
    else:
        meta['replayed_at'] = timestamp
        meta['replayed_from'] = {'path': str(Path(args.log)), 'sha256': meta['log_sha256']}

    try:
        reject_output_links(out_dir)
    except ValueError as exc:
        print(str(exc),file=sys.stderr)
        return 2
    # 外部采集命令也可能新建别名；落盘前再次检查，不能只检查启动时的目录。
    if sections_dir.is_symlink() or (sections_dir.exists() and not sections_dir.is_dir()) or any(p.is_symlink() for p in managed_files) or managed_outputs_overlap():
        print('managed outputs overlap or changed into a symlink/non-directory', file=sys.stderr)
        return 2
    for p in list(watched) + inputs:
        for target in managed_files:
            inplace_log = args.log and p.resolve() == Path(args.log).resolve() == (out_dir/'serial.log').resolve() and target.name == 'serial.log' and p not in watched
            if (aliases(p, target) and not inplace_log) or p.resolve().is_relative_to(sections_dir.resolve()):
                print('managed output aliases input at write time', file=sys.stderr)
                return 2
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    atomic_write(out_dir / 'serial.log', raw_log)

    sections_dir = out_dir / "sections"
    if sections_dir.exists():
        shutil.rmtree(sections_dir)
    sections_dir.mkdir()
    for result in results:
        key = (result["group"], result["libc"] or args.libc)
        section = sections.get(key)
        if section is not None:
            (sections_dir / f"{result['group']}-{result['libc'] or args.libc}.log").write_text(
                section.text, encoding="utf-8"
            )

    write_summary(out_dir, meta, results, totals)

    print(json.dumps(totals, ensure_ascii=False))
    return 0 if meta['execution_valid'] and all(r["status"] == "PASS" for r in results) else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError,OSError) as exc:
        print(f'runner: {exc}', file=sys.stderr)
        raise SystemExit(2)
