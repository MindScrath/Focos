"""构建 FocOS，挂载固定比赛镜像并记录 QEMU 原始串口。"""
import argparse
import ctypes
import errno
import fcntl
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
FIRMWARE = Path("/usr/share/qemu/opensbi-riscv64-generic-fw_dynamic.bin")
LOADER_VARIABLES = {"LD_LIBRARY_PATH", "NIX_LD", "NIX_LD_LIBRARY_PATH"}
IMAGE_SHA = "95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


class ImageError(ValueError):
    pass


def host_environment():
    keys = {"HOME", "PATH", "TMPDIR", "TMP", "TEMP", "LANG", "LANGUAGE",
            "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "CARGO_HOME", "RUSTUP_HOME"}
    return {key: value for key, value in os.environ.items()
            if key in keys or key in LOADER_VARIABLES or key.startswith("LC_")}


def source_state(out, err=None):
    paths = ["--", "."]
    if out.is_relative_to(ROOT):
        paths.append(":(exclude,literal)" + str(out.relative_to(ROOT)))

    env = host_environment()

    def git(*args):
        return command_output(["git", *args], err, env)

    for entry in git("ls-files", "-v", "-z", *paths).split(b"\0"):
        if entry and (entry[:1].islower() or entry[:1] == b"S"):
            raise RuntimeError(f"Git 索引标志隐藏了工作区状态：{os.fsdecode(entry[2:])}")

    revision = git("rev-parse", "HEAD").strip().decode("ascii")
    status = git("status", "--porcelain=v1", "-z", "--untracked-files=all", *paths)
    signature = hashlib.sha256(status)
    if status:
        signature.update(git("diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD", *paths))
        for name in git("ls-files", "--others", "--exclude-standard", "-z", *paths).split(b"\0"):
            if name:
                path = ROOT / os.fsdecode(name)
                if path.is_symlink():
                    signature.update(hashlib.sha256(os.fsencode(os.readlink(path))).digest())
                elif path.is_file():
                    signature.update(bytes.fromhex(digest(path)))
    return {"revision": revision, "dirty": bool(status), "worktree_sha256": signature.hexdigest()}


def command_output(cmd, err, env=None):
    with tempfile.TemporaryFile() as output:
        with child(cmd, stdout=output, stderr=err, env=env) as proc:
            if proc.wait():
                raise subprocess.CalledProcessError(proc.returncode, cmd)
        output.seek(0)
        return output.read()


def probe_version(cmd, err, env=None):
    version = command_output(cmd, err, env).decode("utf-8", errors="backslashreplace").strip()
    if not version:
        raise RuntimeError(f"{cmd[0]} 未返回版本信息")
    return version


def copy_image(image, dest):
    try:
        source = image.open("rb")
    except OSError as exc:
        raise ImageError(f"无法读取镜像 {image}：{exc}") from exc
    with source, dest.open("xb") as target:
        while True:
            try:
                data = source.read(1024 * 1024)
            except OSError as exc:
                raise ImageError(f"无法读取镜像 {image}：{exc}") from exc
            if not data:
                break
            if data.count(0) == len(data):
                target.seek(len(data), os.SEEK_CUR)
            else:
                target.write(data)
        target.truncate()
    dest.chmod(0o400)


class CleanupError(RuntimeError):
    pass


class Terminated(Exception):
    pass


def interrupt(signum, frame):
    if signum == signal.SIGTERM:
        raise Terminated("收到 SIGTERM")
    raise KeyboardInterrupt


def direct_children():
    try:
        children = Path("/proc/thread-self/children").read_bytes()
    except FileNotFoundError:
        # Some kernels omit the children file. waitid rejects non-children,
        # and WNOWAIT preserves exit status for Popen and the cleanup loop.
        children_pids = set()
        for name in os.listdir("/proc"):
            if name.isdecimal():
                pid = int(name)
                try:
                    os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                except ChildProcessError:
                    continue
                children_pids.add(pid)
        return children_pids
    return {int(pid) for pid in children.split()}


def has_children():
    # Only ECHILD proves that the kernel has no child left for us to reap.
    try:
        os.waitid(os.P_ALL, 0, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    except ChildProcessError:
        return False
    return True


@contextmanager
def child(cmd, **kwargs):
    # Linux reparents orphaned grandchildren to this runner, so waitpid can
    # release their process table entries even if the container init does not.
    pr_set_child_subreaper = 36
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(pr_set_child_subreaper, 1, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    pending_signal = None
    running = False

    def handle(signum, frame):
        nonlocal pending_signal
        if running:
            interrupt(signum, frame)
        pending_signal = signum

    previous = {sig: signal.signal(sig, handle) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if has_children():
            raise RuntimeError("入口已有子进程，无法区分本次命令的后代")
        proc = subprocess.Popen(cmd, cwd=ROOT, start_new_session=True,
                                stdin=subprocess.DEVNULL, **kwargs)
        try:
            running = True
            if pending_signal:
                interrupt(pending_signal, None)
            yield proc
        finally:
            running = False
            original = sys.exc_info()[1]
            try:
                reap(proc)
            except (RuntimeError, OSError) as exc:
                detail = f"清理失败：{exc}"
                if original is not None:
                    reason = "用户中断" if isinstance(original, KeyboardInterrupt) else str(original)
                    detail = f"{reason}；{detail}"
                elif proc.returncode is not None:
                    detail = f"{cmd[0]} 进程退出 {proc.returncode}；{detail}"
                if pending_signal:
                    detail += f"；收到 {signal.Signals(pending_signal).name}"
                raise CleanupError(detail) from exc
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    if pending_signal:
        interrupt(pending_signal, None)


def reap(proc):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, sig)
            except ProcessLookupError:
                pass
        until = time.monotonic() + 5
        while time.monotonic() < until:
            # Once their parents exit, even detached descendants are adopted
            # by this single-threaded runner. Repeat for successive generations.
            if proc.poll() is not None:
                if not has_children():
                    return
                adopted = direct_children()
                for pid in adopted:
                    if os.waitpid(pid, os.WNOHANG)[0] == 0:
                        os.kill(pid, sig)
            time.sleep(0.05)
    raise RuntimeError("仍无法确认子进程已全部回收")


@contextmanager
def display_writer(screen, remaining):
    fd = screen.fileno()
    was_blocking = os.get_blocking(fd)
    # poll also accepts terminals, regular files and /dev/null.
    with selectors.PollSelector() as writable:
        writable.register(fd, selectors.EVENT_WRITE)

        def write(data):
            pending = memoryview(data)
            while pending:
                if writable.select(remaining()):
                    try:
                        count = os.write(fd, pending)
                    except BlockingIOError:
                        continue
                    if not count:
                        raise OSError("显示输出写入停滞")
                    pending = pending[count:]

        try:
            os.set_blocking(fd, False)
            yield write
        finally:
            os.set_blocking(fd, was_blocking)


def run_serial(cmd, out, timeout, screen, record=None):
    deadline = time.monotonic() + timeout

    def remaining():
        seconds = deadline - time.monotonic()
        if seconds <= 0:
            raise subprocess.TimeoutExpired(cmd, timeout)
        return seconds

    with (out / "serial.log").open("xb", buffering=0) as log, (out / "qemu.stderr").open("ab") as err:
        with child(cmd, stdout=subprocess.PIPE, stderr=err) as proc:
            with proc.stdout, selectors.DefaultSelector() as selector, display_writer(screen, remaining) as display:
                selector.register(proc.stdout, selectors.EVENT_READ)
                fd = proc.stdout.fileno()
                buffered = None
                while True:
                    if buffered is None and proc.poll() is not None:
                        if record is not None:
                            record["qemu_exit"] = proc.returncode
                        # Descendants can hold or keep writing the same pipe.
                        # Drain exactly the bytes queued when exit is observed.
                        buffered, = struct.unpack("i", fcntl.ioctl(fd, termios.FIONREAD, bytes(4)))
                    if buffered == 0:
                        return proc.returncode
                    if buffered is None and not selector.select(min(0.1, remaining())):
                        continue
                    data = os.read(fd, min(65536, buffered) if buffered is not None else 65536)
                    if not data:
                        rc = proc.wait(timeout=remaining())
                        if record is not None:
                            record["qemu_exit"] = rc
                        return rc
                    if buffered is not None:
                        buffered -= len(data)
                    pending = memoryview(data)
                    while pending:
                        written = log.write(pending)
                        if not written:
                            raise OSError("串口日志写入停滞")
                        pending = pending[written:]
                    display(data)


def report(reason):
    # stderr is optional; a buffered retry at interpreter shutdown can
    # otherwise replace the runner's exit code when the reader has gone.
    try:
        blocking = os.get_blocking(2)
        try:
            os.set_blocking(2, False)
            os.write(2, (str(reason) + "\n").encode("utf-8", "backslashreplace"))
        finally:
            os.set_blocking(2, blocking)
    except OSError:
        pass


class ArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        report(f"{self.format_usage().rstrip()}\n{self.prog}: error: {message}")
        self.exit(2)


def prepare():
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
    try:
        image = args.image.resolve(strict=True)
    except (RuntimeError, OSError) as exc:
        parser.error(f"无法解析镜像路径：{exc}")
    if args.timeout <= 0 or not image.is_file():
        parser.error("需要有效的镜像文件和正整数超时秒数")
    try:
        out = args.out.resolve()
    except (OSError, RuntimeError) as exc:
        parser.error(f"无法解析输出路径：{exc}")
    try:
        out.mkdir(mode=0o700, parents=True, exist_ok=False)
    except FileExistsError:
        parser.error("输出目录已存在")
    except OSError as exc:
        if exc.errno in (errno.ELOOP, errno.ENOTDIR):
            parser.error(f"无效输出路径：{exc}")
        report(exc)
        raise SystemExit(1)
    return args, image, out


def run(args, image, out, record):
    private_image = out / "sdcard-rv.img"
    copy_image(image, private_image)
    record["image_sha256"] = digest(private_image)
    if record["image_sha256"] != IMAGE_SHA:
        raise ImageError("镜像 SHA-256 不匹配")
    with (out / "build.log").open("xb") as log:
        before = source_state(out, log)
        record.update(before)
        target_dir = out / "cargo-target"
        kernel = target_dir / "riscv64gc-unknown-none-elf/release/focos-kernel"
        compiler = os.environ.get("RUSTC") or os.environ.get("CARGO_BUILD_RUSTC") or "rustc"
        record["build_env"] = {
            "CARGO_TARGET_DIR": str(target_dir), "RUSTC": compiler,
            "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
        }
        for key, value in os.environ.items():
            if (
                key in LOADER_VARIABLES
                or key == "RUSTUP_TOOLCHAIN"
                or key.startswith("CARGO_PROFILE_RELEASE_")
                or key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS")
                or (key.startswith("CARGO_TARGET_") and key.endswith("_RUSTFLAGS"))
            ):
                record["build_env"][key] = value
        build_env = host_environment() | record["build_env"]
        for tool, executable in (("rustc", compiler), ("cargo", "cargo")):
            record[f"{tool}_version"] = probe_version(
                [executable, "--version", "--verbose"], log, build_env)
        with child(record["build"], env=build_env,
                   stdout=log, stderr=subprocess.STDOUT) as proc:
            if proc.wait():
                raise RuntimeError("构建失败，见 build.log")
        after = source_state(out, log)
    if after != before:
        record["source_after_build"] = after
        raise RuntimeError("构建期间源码状态发生变化，请使用新的输出目录重试")
    record["kernel_sha256"] = digest(kernel)
    firmware = out / FIRMWARE.name
    with FIRMWARE.open("rb") as source, firmware.open("xb") as target:
        shutil.copyfileobj(source, target)
    firmware.chmod(0o400)
    record["firmware_sha256"] = digest(firmware)
    cmd = ["qemu-system-riscv64", "-machine", "virt", "-kernel", str(kernel),
           "-m", "1G", "-smp", "1", "-display", "none", "-monitor", "none",
           "-serial", "stdio", "-bios", str(firmware), "-no-reboot",
           "-drive", "file=" + str(private_image).replace(",", ",,") + ",if=none,format=raw,id=x0,snapshot=on",
           "-device", "virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0",
           "-device", "virtio-net-device,netdev=net", "-netdev", "user,id=net", "-rtc", "base=utc"]
    record["command"] = cmd
    with (out / "qemu.stderr").open("xb") as err:
        version = probe_version([cmd[0], "--version"], err)
    record["qemu_version"] = version.splitlines()[0]
    rc = run_serial(cmd, out, args.timeout, sys.stdout.buffer, record)
    record["reason"] = f"QEMU 进程退出 {rc}"
    return int(rc != 0)


def main():
    signals = {signal.SIGTERM, signal.SIGINT}
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, signals)
    active = False
    received = set()

    def handle(signum, frame):
        received.add(signum)
        if active and not isinstance(sys.exception(), CleanupError):
            interrupt(signum, frame)

    for sig in signals:
        signal.signal(sig, handle)
    try:
        args, image, out = prepare()
    except SystemExit:
        pending = signal.sigpending()
        if signal.SIGTERM in pending:
            return 143
        if signal.SIGINT in pending:
            return 130
        raise

    record = {"image": str(image), "timeout_seconds": args.timeout, "build": ["make", "build"]}
    code = 1
    cleanup_failed = False
    try:
        try:
            active = True
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask - signals)
            code = run(args, image, out, record)
        finally:
            # Leave the interruptible phase inside the exception boundary.
            active = False
    except ImageError as exc:
        code, record["reason"] = 2, str(exc)
    except subprocess.TimeoutExpired as exc:
        code, record["reason"] = 124, str(exc)
    except Terminated as exc:
        code, record["reason"] = 143, str(exc)
    except KeyboardInterrupt:
        code, record["reason"] = 130, "用户中断"
    except CleanupError as exc:
        cleanup_failed = True
        record["reason"] = str(exc)
    except (OSError, RuntimeError, subprocess.SubprocessError) as exc:
        record["reason"] = str(exc)
    record["exit"] = code
    path = out / "run.json"
    try:
        path.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
        report(record["reason"])
        # Commit the signal decision after the first write and diagnostic.
        # Signals after this snapshot belong to the already-finished run.
        signal.pthread_sigmask(signal.SIG_BLOCK, signals)
        pending = received | signal.sigpending()
        if pending & signals:
            sig = signal.SIGTERM if signal.SIGTERM in pending else signal.SIGINT
            reason = "收到 SIGTERM" if sig == signal.SIGTERM else "用户中断"
            if cleanup_failed:
                if reason not in record["reason"]:
                    record["reason"] += f"；{reason}"
            else:
                code = 128 + sig
                record.update(exit=code, reason=reason)
            path.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    except OSError as exc:
        report(exc)
        return 1
    return code


if __name__ == "__main__":
    sys.exit(main())
