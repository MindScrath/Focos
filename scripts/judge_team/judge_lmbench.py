"""按冻结源码的完整标签/单位判 lmbench；无标签数值必须绑定 shell trace。

格式依据 testsuits-for-oskernel@8b58dd1 的 lmbench_src/src。
旧日志缺少 trace 时，不猜测 lat_mmap 与四个读带宽指标的归属。
"""
import math
import re
import shlex
import sys

from common import NUMBER, emit, expected, positive
from lmbench_trace import command_index

names = expected('lmbench')
labels = [r'Simple syscall', r'Simple read', r'Simple write', r'Simple stat', r'Simple fstat',
          r'Simple open/close', r"Select on 100 fd's", r'Signal handler installation',
          r'Signal handler overhead', r'Protection fault', r'Pipe latency',
          r'(?:Static )?Process fork\+exit', r'(?:Static )?Process fork\+execve',
          r'(?:Static )?Process fork\+/bin/sh -c']
patterns = [(index, re.compile(rf'{label}:\s*({NUMBER}) microseconds'), re.compile(rf'{label}:'))
            for index, label in enumerate(labels)]
patterns += [(14, re.compile(rf'File /var/tmp/XXX write bandwidth:\s*({NUMBER}) KB/sec'), re.compile(r'File /var/tmp/XXX write bandwidth:')),
             (15, re.compile(rf'Pagefaults on /var/tmp/XXX:\s*({NUMBER}) microseconds'), re.compile(r'Pagefaults on /var/tmp/XXX:')),
             (17, re.compile(rf'Pipe bandwidth:\s*({NUMBER}) MB/sec'), re.compile(r'Pipe bandwidth:'))]
found = {}
context = None
measured = False
pending = None
header_expected = False


def start_attempt(index, *, traced=False):
    global context, pending, measured, header_expected
    context = index
    values = found.setdefault(names[index], [])
    pending = len(values)
    values.append(False)
    measured = False
    header_expected = traced and index in (18,23)


def observe(index, ok):
    global measured
    values = found.setdefault(names[index], [])
    if context == index and pending is not None:
        if measured:
            values.append(False)
        else:
            values[pending] = ok
        measured = True
    else:
        values.append(ok)


for raw in sys.stdin.read().splitlines():
    line = raw.strip()
    if not line:
        continue
    if line.startswith('+ '):
        context = None
        pending = None
        header_expected = False
        try:
            args = shlex.split(line[2:])
        except ValueError:
            continue
        if args and args[0] == './lmbench_all':
            context = command_index(args[1:])
        if context is not None:
            start_attempt(context,traced=True)
        continue
    if line in ('file system latency','context switch overhead'):
        index = 18 if line == 'file system latency' else 23
        # trace + 首个表头属于同一次尝试；无 trace 或重复表头各自预留失败观测。
        if context == index and pending is not None and header_expected and not measured:
            header_expected = False
        else:
            start_attempt(index)
        continue
    # 已经出现表格数据或其他输出，后来的表头不能再借用命令预留项。
    header_expected = False
    matched = False
    for index, pattern, prefix in patterns:
        match = pattern.fullmatch(line)
        if prefix.match(line):
            observe(index, bool(match and positive(match[1])))
            matched = True
            break
    if matched:
        context = None
        pending = None
        header_expected = False
        continue
    if context in (16, 19, 20, 21, 22):
        match = re.fullmatch(rf'({NUMBER})\s+({NUMBER})', line)
        if match:
            ok = math.isclose(float(match[1]), 0.524288, rel_tol=0, abs_tol=0.0000005) and positive(match[2])
            observe(context, ok)
            # 每次命令预留失败记录；结果非法、缺失或重复都不能被重试覆盖。
        else:
            observe(context, False)
            context = None
    elif context == 18:
        match = re.fullmatch(r'10k\s+(\d+)\s+(\d+)\s+(\d+)', line)
        if match:
            observe(18, all(int(n) > 0 for n in match.groups()))
        elif not re.fullmatch(r'\d+k\s+\d+\s+\d+\s+\d+', line):
            observe(18, False)
            context = None
    elif context == 23:
        match = re.fullmatch(rf'96\s+({NUMBER})', line)
        if match:
            observe(23, positive(match[1]))
        elif not (re.fullmatch(r'"size=32k ovr=\d+(?:\.\d+)?', line)
                  or re.fullmatch(rf'(?:2|4|8|16|24|32|64)\s+{NUMBER}', line)):
            observe(23, False)
            context = None
emit(names, found)
