"""Check board error output without a serial device or a Rust toolchain."""
import argparse
from contextlib import redirect_stderr, redirect_stdout
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "scripts/dev"
loader = SourceFileLoader("bluetemp_dev_board_info", str(SOURCE))
spec = spec_from_loader(loader.name, loader)
dev = module_from_spec(spec)
loader.exec_module(dev)


def process_result(returncode, stdout="", stderr=""):
    """Model subprocess.run, including its check argument."""
    def execute(command, **kwargs):
        result = subprocess.CompletedProcess(command, returncode, stdout, stderr)
        if kwargs.get("check", False):
            result.check_returncode()
        return result
    return execute


class BoardInfoTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        root_patch = patch.object(dev, "ROOT", self.root)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        for context in [redirect_stdout(self.stdout), redirect_stderr(self.stderr)]:
            context.__enter__()
            self.addCleanup(context.__exit__, None, None, None)

    def test_run_fails_by_default(self):
        with self.assertRaises(subprocess.CalledProcessError) as caught:
            dev.run([sys.executable, "-c", "raise SystemExit(7)"], capture_output=True, text=True)
        self.assertEqual(caught.exception.returncode, 7)

    def test_run_can_defer_exit_check(self):
        result = dev.run([sys.executable, "-c", "raise SystemExit(7)"],
                         capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 7)
        with self.assertRaises(subprocess.CalledProcessError):
            result.check_returncode()

    def test_board_success_prints_and_saves_both_streams(self):
        stdout = "Chip type: ESP32 (revision v3.1)\nFlash size: 4MB\n"
        stderr = "Connection log\n"
        with patch.object(dev.subprocess, "run", side_effect=process_result(0, stdout, stderr)) as child:
            result = dev.board_info("/dev/cu.TEST", manual=True)
        self.assertEqual(result, stdout + stderr)
        self.assertEqual(self.stdout.getvalue(), result)
        self.assertEqual(self.stderr.getvalue(), "")
        self.assertEqual((self.root / "target/board-info.txt").read_text(), result)
        command = child.call_args.args[0]
        self.assertEqual(command[command.index("--before") + 1], "no-reset")
        self.assertEqual(command[command.index("--after") + 1], "no-reset")
        self.assertIn("--no-stub", command)
        dev.verify_board(result)

    def test_board_failure_prints_and_saves_both_streams(self):
        stdout = "Connecting...\n"
        stderr = "Simulated connection failure\n"
        with patch.object(dev.subprocess, "run", side_effect=process_result(1, stdout, stderr)):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                dev.board_info("/dev/cu.TEST", manual=True)
        self.assertEqual(caught.exception.returncode, 1)
        self.assertEqual(caught.exception.stdout, stdout)
        self.assertEqual(caught.exception.stderr, stderr)
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertEqual(self.stderr.getvalue(), stdout + stderr)
        self.assertEqual((self.root / "target/board-info.txt").read_text(), stdout + stderr)

    def test_board_failure_replaces_stale_log(self):
        (self.root / "target").mkdir()
        log = self.root / "target/board-info.txt"
        log.write_text("Old successful board check\n")
        with patch.object(dev.subprocess, "run", side_effect=process_result(1, "", "New failure\n")):
            with self.assertRaises(subprocess.CalledProcessError):
                dev.board_info("/dev/cu.TEST")
        self.assertEqual(log.read_text(), "New failure\n")
        self.assertEqual(self.stderr.getvalue(), "New failure\n")

    def test_board_failure_with_empty_output_still_fails(self):
        (self.root / "target").mkdir()
        log = self.root / "target/board-info.txt"
        log.write_text("Old successful board check\n")
        with patch.object(dev.subprocess, "run", side_effect=process_result(1)):
            with self.assertRaises(subprocess.CalledProcessError):
                dev.board_info("/dev/cu.TEST")
        self.assertEqual(log.read_text(), "")

    def test_flash_does_not_write_after_board_failure(self):
        # Valid-looking data must not override a failed process exit status.
        output = "Chip type: ESP32 (revision v3.1)\nFlash size: 4MB\n"
        with patch.object(dev, "build") as build, \
             patch.object(dev.subprocess, "run", side_effect=process_result(1, output, "Failure\n")) as child:
            with self.assertRaises(subprocess.CalledProcessError):
                dev.flash(argparse.Namespace(port="/dev/cu.TEST", manual=True))
        build.assert_called_once_with()
        self.assertEqual(child.call_count, 1)
        self.assertEqual(child.call_args.args[0][1], "board-info")

    def test_automatic_reset_flags_are_unchanged(self):
        with patch.object(dev.subprocess, "run", side_effect=process_result(0, "Board information\n")) as child:
            dev.board_info("/dev/cu.TEST", manual=False)
        command = child.call_args.args[0]
        self.assertNotIn("--before", command)
        self.assertIn("--non-interactive", command)
        self.assertIn("--no-stub", command)
        self.assertEqual(command[command.index("--after") + 1], "no-reset")

    @unittest.skipUnless(os.name == "posix", "The fake tool requires a POSIX executable file.")
    def test_cli_failure_shows_detail_and_exits_nonzero(self):
        script = self.root / "scripts/dev"
        script.parent.mkdir()
        script.write_bytes(SOURCE.read_bytes())
        tool = self.root / ".tools/bin/espflash"
        tool.parent.mkdir(parents=True)
        tool.write_text(
            f"#!{sys.executable}\n"
            "import sys\n"
            "print('Connecting...')\n"
            "print('Simulated connection failure', file=sys.stderr)\n"
            "raise SystemExit(1)\n"
        )
        tool.chmod(0o755)
        result = subprocess.run(
            [sys.executable, str(script), "board-info", "--port", "/dev/cu.TEST", "--manual"],
            capture_output=True, text=True, check=False, timeout=10,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Simulated connection failure", result.stderr)
        self.assertIn("Error:", result.stderr)
        self.assertEqual(
            (self.root / "target/board-info.txt").read_text(),
            "Connecting...\nSimulated connection failure\n",
        )


if __name__ == "__main__":
    unittest.main()
