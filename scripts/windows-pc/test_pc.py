"""Tests for scripts/windows-pc/pc that need no PC: its commands and its config.

Run by scripts/gates.sh. Each test gives `pc` its own state directory, so the
real one under ~/.local/share is never read.
"""

import os
import subprocess
import tempfile
import unittest

PC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pc")


class PcTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.state = os.path.join(self.tmp.name, "state")
        os.mkdir(self.state)
        self.key = os.path.join(self.tmp.name, "key")
        with open(self.key, "w") as f:
            f.write("not a real key\n")

    def tearDown(self):
        self.tmp.cleanup()

    def pc(self, *args, state=None):
        env = dict(os.environ, WN13_STATE_DIR=state or self.state)
        return subprocess.run(
            [PC, *args], env=env, capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL
        )

    def write(self, name, text):
        with open(os.path.join(self.state, name), "w") as f:
            f.write(text)

    def configure(self, text, known_hosts=True):
        self.write("config", text)
        if known_hosts:
            self.write("known_hosts", "win11-test ssh-ed25519 AAAA\n")

    def assert_refused(self, result, message, code=1):
        self.assertEqual(result.returncode, code, result.stderr)
        self.assertIn(message, result.stderr)

    def test_usage_lists_the_commands_and_no_machine_control(self):
        result = self.pc()
        self.assertEqual(result.returncode, 2)
        for command in ("setup", "run", "screen", "push", "ssh", "selftest", "status", "uninstall"):
            self.assertIn(f"pc {command}", result.stderr)
        for command in ("stop", "reset", "key", "build", "start"):
            self.assertNotIn(f"pc {command}", result.stderr)
            self.assertEqual(self.pc(command).returncode, 2, command)

    def test_relative_state_dir_is_refused(self):
        self.assert_refused(self.pc("status", state="relative/dir"), "absolute path", code=2)

    def test_missing_config_is_refused(self):
        self.assert_refused(self.pc("status"), "no config")

    def test_valid_config_reaches_the_command(self):
        # The usage error comes from `pc run` itself, after the config was read.
        self.configure(f"# the test PC\n\nhost=10.0.0.7\nkey={self.key}\n")
        self.assert_refused(self.pc("run"), "usage: pc run")

    def test_config_without_final_newline_is_read(self):
        self.configure(f"host=win11-test.lan\nkey={self.key}")
        self.assert_refused(self.pc("run"), "usage: pc run")

    def test_bad_configs_are_refused(self):
        cases = {
            "unknown key": (f"host=a\nkey={self.key}\nport=22\n", "not a host= or key= line"),
            "spaces around =": (f"host = a\nkey={self.key}\n", "not a host= or key= line"),
            "option as host": (f"host=-oProxyCommand=x\nkey={self.key}\n", "host must be"),
            "user in host": (f"host=root@a\nkey={self.key}\n", "host must be"),
            "host twice": (f"host=a\nhost=b\nkey={self.key}\n", "host is given twice"),
            "relative key": ("host=a\nkey=key\n", "key must be an absolute path"),
            "key with a space": ("host=a\nkey=/tmp/my key\n", "key must be an absolute path"),
            "key twice": (f"host=a\nkey={self.key}\nkey={self.key}\n", "key is given twice"),
            "no host": (f"key={self.key}\n", "no host= line"),
            "no key": ("host=a\n", "no key= line"),
            "missing key file": (f"host=a\nkey={self.key}.gone\n", "cannot read the key"),
            "empty value": (f"host=\nkey={self.key}\n", "not a host= or key= line"),
        }
        for name, (text, message) in cases.items():
            with self.subTest(name):
                self.configure(text)
                self.assert_refused(self.pc("status"), message)

    def test_bad_password_files_are_refused_before_the_pc_is_reached(self):
        # The config names an address nothing answers at: a refusal that
        # names the password file came before any connection.
        cases = {
            "too short": "abc123\n",
            "two lines": "abcdefghijklmnop1A\nabcdefghijklmnop1A\n",
            "a symbol": "abcdefghijklmnop1A!\n",
            "empty": "",
            "a bad line before a good one": "bad line\nabcdefghijklmnop1A",
            "no newline": "abcdefghijklmnop1A",
            "a blank line after": "abcdefghijklmnop1A\n\n",
            "a NUL for its newline": "abcdefghijklmnop1A\0",
            "a NUL inside": "A\0bcdefghijklmnopq\n",
        }
        for name, text in cases.items():
            with self.subTest(name):
                self.configure(f"host=192.0.2.1\nkey={self.key}\n")
                self.write("password", text)
                self.assert_refused(self.pc("setup"), "must hold one line of 16 to 128 letters and digits")
                mode = os.stat(os.path.join(self.state, "password")).st_mode & 0o777
                self.assertEqual(mode, 0o600, name)

    def test_missing_known_hosts_is_refused(self):
        self.configure(f"host=a\nkey={self.key}\n", known_hosts=False)
        self.assert_refused(self.pc("status"), "known_hosts")


if __name__ == "__main__":
    unittest.main()
