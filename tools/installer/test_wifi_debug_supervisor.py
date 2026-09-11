"""Execute the shipped Wi-Fi debug lifecycle scripts in a bounded sandbox."""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest


STAGE = Path(__file__).resolve().parent / 'wifi-stage'


def extract(text, name):
    start = text.index(f'{name}() {{')
    depth, index = 0, start
    while index < len(text):
        if text[index] == '{':
            depth += 1
        elif text[index] == '}':
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
        index += 1
    raise ValueError(f'unterminated shell function {name}')


def wait_for(path, expected, seconds=3):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if path.exists() and path.read_text().strip() == expected:
            return
        time.sleep(.02)
    raise AssertionError(f'timed out waiting for {path}={expected!r}')


class Harness:
    def __init__(self, root):
        self.root = root
        self.busybox = shutil.which('busybox')
        self.shell = [self.busybox, 'sh'] if self.busybox else ['sh']
        self.runner = self.busybox or shutil.which('sh')
        self.wrapper = root / 'busybox'
        self.calls = root / 'killall.calls'
        self.wrapper.write_text(
            '#!/bin/sh\n'
            'if [ "$1" = dmesg ]; then echo "[debug kernel tail]"; exit 0; fi\n'
            'if [ "$1" = killall ]; then\n'
            f'  printf "%s\\n" "$*" >> {self.calls}\n'
            '  exit 0\n'
            'fi\n'
            + (f'exec {self.runner} "$@"\n' if self.busybox else 'exec "$@"\n'))
        self.wrapper.chmod(0o755)

    def run(self, body):
        script = self.root / 'test.sh'
        script.write_text(body)
        return subprocess.run(self.shell + [str(script)], text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=5, check=False)

    def marker(self):
        path = self.root / 'couch-wifi-debug.retry'
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


class DebugSupervisorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.harness = Harness(self.root)
        self.supervisor = STAGE.joinpath('debug-supervisor').read_text()
        self.wifi_init = STAGE.joinpath('wifi-init').read_text()

    def tearDown(self):
        self.directory.cleanup()

    def staged_supervisor(self, mode):
        fail = extract(self.wifi_init, 'fail').replace('/tmp', str(self.root))
        init = self.root / 'couch-wifi-init'
        if mode == 'after':
            init.write_text(
                f'#!{self.harness.runner} sh\nBB={self.harness.wrapper}\n'
                f'echo start >> {self.root / "starts"}\n'
                'starting_supplicant=0\n'
                f'{fail}\n'
                'fail supplicant-socket-timeout\n')
        else:
            init.write_text(
                f'#!{self.harness.runner} sh\nBB={self.harness.wrapper}\n'
                f'echo start >> {self.root / "starts"}\n'
                "trap 'exit 0' TERM\n"
                'while true; do $BB sleep 1; done\n')
        init.chmod(0o755)
        text = self.supervisor.replace('/tmp', str(self.root))
        text = text.replace('/bin/couch-wifi-init', str(init))
        text = text.replace('BB=/bin/busybox', f'BB={self.harness.wrapper}')
        script = self.root / 'debug-supervisor'
        script.write_text(text)
        script.chmod(0o755)
        return script

    def run_retry(self, mode):
        process = subprocess.Popen(self.harness.shell + [str(self.staged_supervisor(mode))])
        try:
            wait_for(self.root / 'couch-wifi-debug.generation', '1')
            if mode == 'after':
                wait_for(self.root / 'couch-wifi.status', 'failed')
            self.harness.marker()
            wait_for(self.root / 'couch-wifi-debug.generation', '2')
            time.sleep(.1)
            self.assertEqual((self.root / 'couch-wifi-debug.generation').read_text().strip(), '2')
            self.assertEqual((self.root / 'starts').read_text().splitlines(), ['start', 'start'])
            self.assertFalse((self.root / 'couch-wifi-debug.retry').exists())
            lifecycle = (self.root / 'couch-wifi-debug.lifecycle').read_text()
            self.assertIn('supervisor=couch-wifi-debug-supervisor-v1', lifecycle)
            self.assertIn('generation=2', lifecycle)
            self.assertIn('retry=consumed', lifecycle)
        finally:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    @unittest.skipUnless(shutil.which('busybox'), 'requires BusyBox ash lifecycle semantics')
    def test_marker_after_failed_worker_advances_once(self):
        self.run_retry('after')

    @unittest.skipUnless(shutil.which('busybox'), 'requires BusyBox ash lifecycle semantics')
    def test_marker_during_running_worker_advances_once_with_fixed_cleanup(self):
        self.run_retry('during')
        self.assertEqual(self.harness.calls.read_text().splitlines(),
                         ['killall wpa_supplicant wmt_launcher wmt_loader'])
        self.assertNotIn('couch-installer-probe', self.supervisor)

    @unittest.skipUnless(os.name == 'posix', 'requires POSIX shell startup semantics')
    def test_debug_startup_evidence_prioritizes_wpa_and_liveness(self):
        marker = self.root / 'debug-mode'
        marker.touch()
        startup = self.root / 'couch-wpa-startup.log'
        startup.write_text('wpa -dd startup evidence\\nquote " slash \\\\ newline\\n')
        (self.root / 'couch-wpa').mkdir()
        debug = extract(self.wifi_init, 'debug_startup_evidence').replace('/tmp', str(self.root))
        fail = extract(self.wifi_init, 'fail').replace('/tmp', str(self.root))
        debug = debug.replace('/etc/couch-wifi-debug-mode', str(marker))
        fail = fail.replace('/etc/couch-wifi-debug-mode', str(marker))
        body = (f'BB={self.harness.wrapper}\n'
                f'{debug}\n{fail}\n'
                'starting_supplicant=1\n'
                f'touch {marker}\n'
                'supplicant=999999\n'
                'fail supplicant-socket-timeout\n')
        result = self.harness.run(body)
        self.assertEqual(result.returncode, 1)
        evidence = (self.root / 'couch-wifi-debug.log').read_text()
        self.assertLessEqual(len(evidence.encode()), 4096)
        self.assertIn('wpa -dd startup evidence', evidence)
        self.assertIn('no longer running at failure', evidence)
        self.assertIn('WiFi control directory:', evidence)
        self.assertIn('[debug kernel tail]', evidence)

    @unittest.skipUnless(os.name == 'posix', 'requires POSIX shell startup semantics')
    def test_only_debug_mode_adds_detailed_supplicant_verbosity(self):
        marker = self.root / 'debug-mode'
        binary = self.root / 'wpa_supplicant'
        binary.write_text(f'#!/bin/sh\nprintf "%s\\n" "$*" >> {self.root / "wpa.args"}\n')
        binary.chmod(0o755)
        start = extract(self.wifi_init, 'start_supplicant')
        start = start.replace('/etc/couch-wifi-debug-mode', str(marker))
        start = start.replace('/sbin/wpa_supplicant', str(binary))
        normal = self.harness.run(f'BB={self.harness.wrapper}\n{start}\nstart_supplicant\n')
        self.assertEqual(normal.returncode, 0)
        self.assertNotIn('-dd', (self.root / 'wpa.args').read_text())
        marker.touch()
        debug = self.harness.run(f'BB={self.harness.wrapper}\n{start}\nstart_supplicant\n')
        self.assertEqual(debug.returncode, 0)
        args = (self.root / 'wpa.args').read_text().splitlines()
        self.assertIn('-dd', args[-1])
        self.assertNotIn('-K', '\n'.join(args))


if __name__ == '__main__':
    unittest.main()
