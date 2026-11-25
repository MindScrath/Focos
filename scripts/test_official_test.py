import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from unittest.mock import patch

from official_test import child, run_serial


class SerialTests(unittest.TestCase):
    def test_raw_bytes_and_separate_diagnostics(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cmd = [sys.executable, "-c", "import os; os.write(1,b'\\xff\\r\\n'); os.write(2,b'error')"]
            with open(os.devnull, "wb") as screen:
                self.assertEqual(run_serial(cmd, root, 3, screen), 0)
            self.assertEqual((root / "serial.log").read_bytes(), b"\xff\r\n")
            self.assertEqual((root / "qemu.stderr").read_bytes(), b"error")

    def test_silent_child_times_out(self):
        with tempfile.TemporaryDirectory() as tmp, open(os.devnull, "wb") as screen:
            with self.assertRaises(subprocess.TimeoutExpired):
                run_serial([sys.executable, "-c", "import time; time.sleep(30)"],
                           Path(tmp), 0.2, screen)

    def test_interrupted_or_failed_output_reaps_vm(self):
        for error in (KeyboardInterrupt, OSError):
            with self.subTest(error=error), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                pidfile = root / "pid"
                cmd = [sys.executable, "-c",
                       "import os,sys,time; from pathlib import Path; "
                       "Path(sys.argv[1]).write_text(str(os.getpid())); "
                       "print('boot', flush=True); time.sleep(30)", str(pidfile)]

                with open(os.devnull, "wb") as screen, patch("official_test.os.write", side_effect=error):
                    with self.assertRaises(error):
                        run_serial(cmd, root, 3, screen)
                self.assertEqual((root / "serial.log").read_bytes(), b"boot\n")
                with self.assertRaises(ProcessLookupError):
                    os.kill(int(pidfile.read_text()), 0)

    def test_short_disk_write_is_reported(self):
        # Linux can accept a prefix without raising until the next write.
        with tempfile.TemporaryDirectory() as tmp:
            probe = """
import os, resource, signal, sys
from pathlib import Path
from official_test import run_serial
signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
resource.setrlimit(resource.RLIMIT_FSIZE, (8, 8))
with open(os.devnull, 'wb') as screen:
    try:
        run_serial([sys.executable, '-c', 'print("0123456789")'],
                   Path(sys.argv[1]), 3, screen)
    except OSError as error:
        import errno
        if error.errno == errno.EFBIG:
            sys.exit(0)
        raise
sys.exit(1)
"""
            result = subprocess.run([sys.executable, "-B", "-c", probe, tmp],
                                    cwd=Path(__file__).resolve().parent)
            self.assertEqual(result.returncode, 0)
            self.assertEqual((Path(tmp) / "serial.log").read_bytes(), b"01234567")

    def test_cleanup_failure_keeps_both_reasons(self):
        for original, text in [(KeyboardInterrupt(), "用户中断"),
                               (subprocess.TimeoutExpired("qemu", 1), "timed out")]:
            with self.subTest(original=original), patch("official_test.reap",
                                                       side_effect=OSError("cleanup error")):
                with self.assertRaises(RuntimeError) as raised:
                    with child([sys.executable, "-c", "pass"]) as proc:
                        proc.wait()
                        raise original
                self.assertIn(text, str(raised.exception))
                self.assertIn("cleanup error", str(raised.exception))

    def test_return_code_survives_failed_cleanup(self):
        record = {}
        with tempfile.TemporaryDirectory() as tmp, open(os.devnull, "wb") as screen:
            with patch("official_test.reap", side_effect=OSError("cleanup failed")):
                with self.assertRaisesRegex(RuntimeError, "进程退出 7；清理失败"):
                    run_serial([sys.executable, "-c", "raise SystemExit(7)"],
                               Path(tmp), 3, screen, record)
        self.assertEqual(record["qemu_exit"], 7)

    def test_sigkill_reaps_orphan_without_proc_children(self):
        # The leader exits only after its helper is ready and ignores TERM.
        cmd = """import os, signal
r, w = os.pipe()
pid = os.fork()
if not pid:
    os.close(r)
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    os.write(w, b'1')
    signal.pause()
else:
    os.close(w)
    os.read(r, 1)
    print(pid)
"""
        read_bytes = Path.read_bytes

        def without_children(path):
            if path == Path("/proc/thread-self/children"):
                raise FileNotFoundError(str(path))
            return read_bytes(path)

        with patch.object(Path, "read_bytes", without_children):
            with child([sys.executable, "-c", cmd], stdout=subprocess.PIPE) as proc:
                with proc.stdout:
                    helper = int(proc.stdout.readline())
                    self.assertEqual(proc.wait(timeout=3), 0)
        with self.assertRaises(ProcessLookupError):
            os.kill(helper, 0)

    def test_stalled_display_times_out(self):
        r, w = os.pipe()
        with os.fdopen(r, "rb"), os.fdopen(w, "wb") as screen, tempfile.TemporaryDirectory() as tmp:
            os.set_blocking(w, False)
            with self.assertRaises(BlockingIOError):
                while True:
                    os.write(w, b"x" * 4096)
            os.set_blocking(w, True)
            with self.assertRaises(subprocess.TimeoutExpired):
                run_serial([sys.executable, "-c", "print('boot')"], Path(tmp), 0.5, screen)
            self.assertTrue(os.get_blocking(w))
            self.assertEqual((Path(tmp) / "serial.log").read_bytes(), b"boot\n")

    def test_second_sigint_during_cleanup(self):
        from official_test import reap

        def interrupt_cleanup(proc):
            os.kill(os.getpid(), signal.SIGINT)
            reap(proc)

        with patch("official_test.reap", side_effect=interrupt_cleanup):
            with self.assertRaises(KeyboardInterrupt):
                with child([sys.executable, "-c", "import time; time.sleep(30)"]) as proc:
                    os.kill(os.getpid(), signal.SIGINT)
        self.assertIsNotNone(proc.returncode)
        with self.assertRaises(ProcessLookupError):
            os.killpg(proc.pid, 0)

    def test_bad_image_records_utf8_reason_in_ascii_locale(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "image"
            image.write_bytes(b"invalid")
            out = root / "run"
            result = subprocess.run(
                [sys.executable, Path(__file__).with_name("official_test.py"),
                 "--image", image, "--out", out],
                env=os.environ | {"LC_ALL": "C", "PYTHONUTF8": "0"},
                capture_output=True)
            self.assertEqual(result.returncode, 2, result.stderr)
            record = json.loads((out / "run.json").read_text(encoding="utf-8"))
            self.assertEqual(record["exit"], 2)
            self.assertEqual(record["reason"], "镜像 SHA-256 不匹配")

    def test_closed_stderr_keeps_bad_image_record(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "bad.img"
            image.write_bytes(b"invalid")
            out = root / "out"
            reader, writer = os.pipe()
            os.close(reader)
            with os.fdopen(writer, "wb") as closed_pipe:
                result = subprocess.run(
                    [sys.executable, Path(__file__).with_name("official_test.py"),
                     "--image", image, "--out", out], stderr=closed_pipe)
            self.assertEqual(result.returncode, 2)
            record = json.loads((out / "run.json").read_bytes())
            self.assertEqual(record["exit"], 2)
            self.assertEqual(record["reason"], "镜像 SHA-256 不匹配")

    def test_parameter_errors_with_closed_stderr(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "image"
            image.touch()
            for args in ([], ["--image", image, "--out", root, "--timeout", "0"],
                         ["--image", root / "missing", "--out", root],
                         ["--image", image, "--out", root]):
                reader, writer = os.pipe()
                os.close(reader)
                with self.subTest(args=args), os.fdopen(writer, "wb") as sink:
                    result = subprocess.run(
                        [sys.executable, Path(__file__).with_name("official_test.py"), *args],
                        stderr=sink)
                    self.assertEqual(result.returncode, 2)

    @unittest.skipIf(os.geteuid() == 0, "root can read mode-000 files")
    def test_unreadable_image_is_input_error(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "image"
            image.touch(mode=0)
            out = root / "out"
            result = subprocess.run(
                [sys.executable, Path(__file__).with_name("official_test.py"),
                 "--image", image, "--out", out], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(json.loads((out / "run.json").read_bytes())["exit"], 2)
            self.assertFalse((out / "build.log").exists())

    def test_image_copy_survives_source_replacement(self):
        from official_test import copy_image
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, dest = root / "input", root / "copy"
            data = b"data" + bytes(2 * 1024 * 1024)
            source.write_bytes(data)
            copy_image(source, dest)
            source.unlink()
            source.write_bytes(b"replacement")
            self.assertEqual(dest.read_bytes(), data)
            self.assertFalse(dest.stat().st_mode & 0o222)

    def test_image_symlink_loop(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "loop"
            image.symlink_to(image)
            result = subprocess.run(
                [sys.executable, Path(__file__).with_name("official_test.py"),
                 "--image", image, "--out", root / "run"], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b"Traceback", result.stderr)

    def test_output_symlink_loop(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "image"
            image.touch()
            loop = root / "loop"
            loop.symlink_to(loop)
            result = subprocess.run(
                [sys.executable, Path(__file__).with_name("official_test.py"),
                 "--image", image, "--out", loop / "run"], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b"Traceback", result.stderr)

    def test_sigterm_at_final_report(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "image"
            image.touch()
            out = root / "out"
            probe = """
import os, signal, sys
import official_test as runner
def terminate(reason):
    os.kill(os.getpid(), signal.SIGTERM)
runner.report = terminate
signal.signal(signal.SIGTERM, runner.interrupt)
sys.exit(runner.main())
"""
            result = subprocess.run([sys.executable, "-B", "-c", probe, "--image",
                                     image, "--out", out], cwd=Path(__file__).parent)
            self.assertEqual(result.returncode, 143)
            self.assertEqual(json.loads((out / "run.json").read_bytes())["exit"], 143)

    def test_termination_during_prepare(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = root / "input"
            image.touch()
            out = root / "out"
            probe = """
import os, signal, sys
from pathlib import Path
import official_test as runner
mkdir = Path.mkdir
def interrupted(path, *args, **kwargs):
    mkdir(path, *args, **kwargs)
    os.kill(os.getpid(), signal.SIGTERM)
Path.mkdir = interrupted
sys.exit(runner.main())
"""
            result = subprocess.run([sys.executable, "-B", "-c", probe, "--image",
                                     image, "--out", out], cwd=Path(__file__).parent,
                                    capture_output=True)
            self.assertEqual(result.returncode, 143, result.stderr)
            self.assertEqual(json.loads((out / "run.json").read_bytes())["exit"], 143)
            self.assertFalse((out / "build.log").exists())

    def test_source_change_in_already_dirty_tree(self):
        from official_test import source_state
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture_env = os.environ.copy()
            def git(*args):
                subprocess.run(["git", "-c", "maintenance.auto=false", "-c", "gc.auto=0", *args], cwd=root, check=True, capture_output=True, env=fixture_env)
            git("init")
            source = root / "source"
            source.write_bytes(b"base")
            git("add", "source")
            git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                "commit", "-m", "base")
            out = root / "run"
            out.mkdir()
            with patch("official_test.ROOT", root), patch.dict(os.environ, GIT_DIR=str(root / "wrong.git")):
                self.assertFalse(source_state(out)["dirty"])
                source.write_bytes(b"first")
                first = source_state(out)
                (out / "build.log").touch()
                self.assertEqual(first, source_state(out))
                source.write_bytes(b"second")
                self.assertNotEqual(first, source_state(out))
                for flag in ("assume-unchanged", "skip-worktree"):
                    with self.subTest(flag=flag):
                        git("update-index", "--" + flag, "source")
                        with self.assertRaisesRegex(RuntimeError, "source"):
                            source_state(out)
                        git("update-index", "--no-" + flag, "source")

    def test_version_probe_reaps_helpers_on_sigterm(self):
        from official_test import Terminated, probe_version
        with tempfile.TemporaryDirectory() as tmp:
            pidfile = Path(tmp) / "helper"
            script = """
import os, signal, subprocess, sys
from pathlib import Path
helper = subprocess.Popen(["sleep", "60"])
Path(sys.argv[1]).write_text(str(helper.pid))
os.kill(os.getppid(), signal.SIGTERM)
signal.pause()
"""
            with self.assertRaises(Terminated):
                probe_version([sys.executable, "-c", script, str(pidfile)], subprocess.DEVNULL)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pidfile.read_text()), 0)

    def test_version_output_in_ascii_locale(self):
        script = r"""
from official_test import probe_version
import subprocess, sys
cmd = [sys.executable, "-c", r"import os; os.write(1, b'version \xc3\xa9\xff')"]
assert probe_version(cmd, subprocess.DEVNULL) == "version \u00e9\\xff"
"""
        subprocess.run([sys.executable, "-B", "-c", script], cwd=Path(__file__).parent,
                       env=os.environ | {"LC_ALL": "C", "PYTHONUTF8": "0"}, check=True)

    def test_git_probe_reaps_helper_on_sigterm(self):
        from official_test import Terminated, source_state
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git = root / "git"
            git.write_text("#!/bin/sh\nsleep 60 &\necho $! > helper\nkill -TERM $PPID\nwait\n")
            git.chmod(0o755)
            with patch("official_test.ROOT", root), patch.dict(
                    os.environ, PATH=tmp + os.pathsep + os.environ["PATH"]):
                with self.assertRaises(Terminated):
                    source_state(root / "out")
            with self.assertRaises(ProcessLookupError):
                os.kill(int((root / "helper").read_text()), 0)

    def test_exited_vm_with_inherited_serial_pipe(self):
        script = """
import sys, subprocess
helper = subprocess.Popen(["sleep", "30"], start_new_session=True)
print(helper.pid, flush=True)
sys.exit(7)
"""
        with tempfile.TemporaryDirectory() as tmp, open(os.devnull, "wb") as screen:
            out = Path(tmp)
            record = {}
            self.assertEqual(run_serial([sys.executable, "-c", script], out, 2, screen, record), 7)
            self.assertEqual(record["qemu_exit"], 7)
            with self.assertRaises(ProcessLookupError):
                os.kill(int((out / "serial.log").read_bytes()), 0)

    def test_existing_child_is_left_running(self):
        with subprocess.Popen(["sleep", "30"]) as existing:
            try:
                with self.assertRaisesRegex(RuntimeError, "已有子进程"):
                    with child(["true"]):
                        self.fail("unexpected launch")
                self.assertIsNone(existing.poll())
            finally:
                existing.terminate()

    def test_empty_proc_listing_does_not_prove_cleanup(self):
        from official_test import direct_children
        scans = []
        def incomplete_first_scan():
            scans.append(True)
            return set() if len(scans) == 1 else direct_children()

        with patch("official_test.direct_children", incomplete_first_scan):
            with child(["sh", "-c", "sleep 30 </dev/null >/dev/null 2>&1 & echo $!"],
                       stdout=subprocess.PIPE) as proc:
                with proc.stdout:
                    helper = int(proc.stdout.readline())
                proc.wait(timeout=3)
        self.assertGreaterEqual(len(scans), 2)
        with self.assertRaises(ProcessLookupError):
            os.kill(helper, 0)

    def test_build_cleanup_failure_stops_entry_and_records_reason(self):
        import json
        script = """
import os, signal, sys
from pathlib import Path
import official_test as runner
runner.IMAGE_SHA = runner.digest(Path(sys.argv[2]))
reap = runner.reap
def fail_after_build(proc):
    reap(proc)
    if proc.args == ["make", "build"]:
        os.kill(os.getpid(), signal.SIGTERM)
        raise OSError("无法确认子进程状态")
runner.reap = fail_after_build
sys.exit(runner.main())
"""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image, out, make = root / "image", root / "output", root / "make"
            image.write_bytes(b"fixture")
            profile = {"CARGO_PROFILE_RELEASE_OPT_LEVEL": "1", "CARGO_PROFILE_RELEASE_LTO": "off"}
            make.write_text("#!/bin/sh\necho \"$CARGO_PROFILE_RELEASE_OPT_LEVEL $CARGO_PROFILE_RELEASE_LTO\"\nexit 7\n")
            make.chmod(0o755)
            compiler = root / "compiler"
            compiler.write_text("#!/bin/sh\necho fixture\n")
            compiler.chmod(0o755)
            result = subprocess.run([sys.executable, "-B", "-c", script, "--image",
                                     str(image), "--out", str(out)],
                                    cwd=Path(__file__).parent, capture_output=True,
                                    env=os.environ | profile | {"PATH": tmp + os.pathsep + os.environ["PATH"],
                                                                "RUSTC": str(compiler)})
            record = json.loads((out / "run.json").read_bytes())
            self.assertEqual((result.returncode, record["exit"]), (1, 1))
            self.assertIn(b"1 off", (out / "build.log").read_bytes().splitlines())
            for key, value in profile.items():
                self.assertEqual(record["build_env"][key], value)
            self.assertIn("构建失败", record["reason"])
            self.assertIn("无法确认子进程状态", record["reason"])
            self.assertIn("SIGTERM", record["reason"])
            self.assertFalse((out / "qemu.stderr").exists())

    def test_signal_during_final_mask_or_record_write(self):
        import json
        probe = """
import os, signal, sys
from pathlib import Path
import official_test as runner
mode = sys.argv.pop(1)
mask = signal.pthread_sigmask
write = Path.write_text
def inject_mask(how, signals):
    if how == signal.SIG_BLOCK and "record" in sys._getframe(1).f_locals:
        os.kill(os.getpid(), signal.SIGTERM)
    return mask(how, signals)
def inject_write(path, *args, **kwargs):
    if path.name == "run.json":
        os.kill(os.getpid(), signal.SIGTERM)
    return write(path, *args, **kwargs)
if mode == "mask":
    signal.pthread_sigmask = inject_mask
else:
    Path.write_text = inject_write
sys.exit(runner.main())
"""
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / "image"
            image.touch()
            for mode in ("mask", "write"):
                with self.subTest(mode=mode):
                    out = Path(tmp) / mode
                    result = subprocess.run([sys.executable, "-B", "-c", probe, mode,
                                             "--image", image, "--out", out],
                                            cwd=Path(__file__).parent, capture_output=True)
                    self.assertEqual(result.returncode, 143, result.stderr)
                    self.assertEqual(json.loads((out / "run.json").read_bytes())["exit"], 143)

    def test_inherited_signal_mask_during_command(self):
        import json
        probe = """
import os, signal, sys
from pathlib import Path
import official_test as runner
sig = int(sys.argv.pop(1))
signal.pthread_sigmask(signal.SIG_BLOCK, {sig, signal.SIGUSR1})
def run(args, image, out, record):
    assert signal.SIGUSR1 in signal.pthread_sigmask(signal.SIG_BLOCK, set())
    script = "echo $$ > pid; kill -%d $PPID; sleep 0.5; touch finished" % sig
    with runner.child(["sh", "-c", script]) as proc:
        proc.wait()
    record["reason"] = "completed"
    return 0
runner.run = run
runner.ROOT = Path.cwd()
sys.exit(runner.main())
"""
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / "image"
            image.touch()
            for sig in (signal.SIGTERM, signal.SIGINT):
                with self.subTest(sig=sig):
                    out = Path(tmp) / sig.name
                    result = subprocess.run([sys.executable, "-B", "-c", probe, str(int(sig)),
                                             "--image", image, "--out", out], cwd=tmp,
                                            env=os.environ | {"PYTHONPATH": str(Path(__file__).resolve().parent)},
                                            capture_output=True, timeout=5)
                    self.assertEqual(result.returncode, 128 + sig, result.stderr)
                    self.assertEqual(json.loads((out / "run.json").read_bytes())["exit"], 128 + sig)
                    self.assertFalse((Path(tmp) / "finished").exists())
                    with self.assertRaises(ProcessLookupError):
                        os.killpg(int((Path(tmp) / "pid").read_text()), 0)

    def test_private_firmware_and_loader_environment(self):
        import hashlib
        import json
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image, firmware, out = root / "image", root / "system-bios", root / "run"
            image.touch()
            firmware.write_bytes(b"original")
            loader = {"LD_LIBRARY_PATH": str(root / "libs"), "NIX_LD": "/test/ld",
                      "NIX_LD_LIBRARY_PATH": str(root / "nix-libs")}
            compiler = root / "compiler"
            compiler.write_text('#!/bin/sh\nprintf "%s\\n" "$LD_LIBRARY_PATH" "$NIX_LD" "$NIX_LD_LIBRARY_PATH"\n')
            make = root / "make"
            make.write_text('#!/bin/sh\nd="$CARGO_TARGET_DIR/riscv64gc-unknown-none-elf/release"\n'
                            'mkdir -p "$d" && touch "$d/focos-kernel"\n')
            qemu = root / "qemu-system-riscv64"
            qemu.write_text("""#!/bin/sh
if [ "$1" = --version ]; then
    echo replacement > "$FIXTURE_BIOS.new" && mv "$FIXTURE_BIOS.new" "$FIXTURE_BIOS"
    echo fixture
    exit 0
fi
while [ "$1" != -bios ]; do shift; done
cat "$2"
""")
            for tool in (compiler, make, qemu):
                tool.chmod(0o755)
            probe = """
import os, sys
from pathlib import Path
import official_test as runner
runner.FIRMWARE = Path(os.environ["FIXTURE_BIOS"])
runner.IMAGE_SHA = runner.digest(Path(sys.argv[2]))
sys.exit(runner.main())
"""
            result = subprocess.run([sys.executable, "-B", "-c", probe, "--image", image,
                                     "--out", out], cwd=Path(__file__).parent, capture_output=True,
                                    env=os.environ | loader | {"PATH": tmp + os.pathsep + os.environ["PATH"],
                                                               "RUSTC": str(compiler), "FIXTURE_BIOS": str(firmware)})
            self.assertEqual(result.returncode, 0, result.stderr)
            record = json.loads((out / "run.json").read_bytes())
            self.assertEqual(record["rustc_version"].splitlines(), list(loader.values()))
            self.assertEqual({k: record["build_env"][k] for k in loader}, loader)
            bios = Path(record["command"][record["command"].index("-bios") + 1])
            self.assertEqual(bios.parent, out)
            self.assertEqual(bios.stat().st_mode & 0o222, 0)
            self.assertEqual(firmware.read_bytes(), b"replacement\n")
            self.assertEqual((out / "serial.log").read_bytes(), b"original")
            self.assertEqual(record["firmware_sha256"], hashlib.sha256(b"original").hexdigest())
