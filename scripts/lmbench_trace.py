"""冻结镜像的 24 条 lmbench 命令；判分上下文与未知名称共用。"""
import shlex

# 顺序与 tests/inventory.json 的 lmbench.subcases 相同。
COMMANDS = (
    ('lat_syscall', '-P', '1', 'null'),
    ('lat_syscall', '-P', '1', 'read'),
    ('lat_syscall', '-P', '1', 'write'),
    ('lat_syscall', '-P', '1', 'stat', '/var/tmp/lmbench'),
    ('lat_syscall', '-P', '1', 'fstat', '/var/tmp/lmbench'),
    ('lat_syscall', '-P', '1', 'open', '/var/tmp/lmbench'),
    ('lat_select', '-n', '100', '-P', '1', 'file'),
    ('lat_sig', '-P', '1', 'install'),
    ('lat_sig', '-P', '1', 'catch'),
    ('lat_sig', '-P', '1', 'prot', 'lat_sig'),
    ('lat_pipe', '-P', '1'),
    ('lat_proc', '-P', '1', 'fork'),
    ('lat_proc', '-P', '1', 'exec'),
    ('lat_proc', '-P', '1', 'shell'),
    ('lmdd', 'label=File /var/tmp/XXX write bandwidth:', 'of=/var/tmp/XXX', 'move=1m', 'fsync=1', 'print=3'),
    ('lat_pagefault', '-P', '1', '/var/tmp/XXX'),
    ('lat_mmap', '-P', '1', '512k', '/var/tmp/XXX'),
    ('bw_pipe', '-P', '1'),
    ('lat_fs', '/var/tmp'),
    ('bw_file_rd', '-P', '1', '512k', 'io_only', '/var/tmp/XXX'),
    ('bw_file_rd', '-P', '1', '512k', 'open2close', '/var/tmp/XXX'),
    ('bw_mmap_rd', '-P', '1', '512k', 'mmap_only', '/var/tmp/XXX'),
    ('bw_mmap_rd', '-P', '1', '512k', 'open2close', '/var/tmp/XXX'),
    ('lat_ctx', '-P', '1', '-s', '32', '2', '4', '8', '16', '24', '32', '64', '96'),
)
COMMAND_INDEX = {command: index for index, command in enumerate(COMMANDS)}


def logged_names(text, names):
    result = set()
    for raw in text.splitlines():
        line = raw.strip()
        if not line.startswith('+ '):
            continue
        try:
            args = shlex.split(line[2:])
        except ValueError:
            if line[2:].startswith('./lmbench_all'):
                result.add('lmbench malformed-trace ' + line[2:])
            continue
        if not args or args[0] != './lmbench_all':
            continue
        command = tuple(args[1:])
        index = COMMAND_INDEX.get(command)
        result.add(names[index] if index is not None else 'lmbench ' + shlex.join(command))
    return result
