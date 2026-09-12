"""Execute the real wifi-init failure paths against stubs.

The stage script needs MTK hardware to run end to end, so these tests extract
its actual shell functions from the file and run them, rather than restating
the logic in a copy that could drift. Absolute /tmp paths are rewritten to a
sandbox so a test run cannot touch a real stage state file.
"""
import os
from pathlib import Path
import re
import shutil
import subprocess
import unittest

STAGE = Path(__file__).resolve().parent/'wifi-stage'/'wifi-init'


def extract(text, name):
    """Return the source of one shell function, by brace depth.

    Braces inside quotes or comments do not nest, so counting them naively
    would end the function early or run off the end of the file. Track quoting
    and comments well enough that a message containing a brace cannot mislead
    the scan.
    """
    start = text.index(f'{name}() {{')
    depth, index, quote, comment = 0, start, None, False
    while index < len(text):
        char = text[index]
        if comment:
            comment = char != '\n'
        elif quote:
            if char == '\\' and quote == '"':
                index += 2
                continue
            if char == quote:
                quote = None
        elif char in '\'"':
            quote = char
        elif char == '#' and (index == 0 or text[index-1] in ' \t\n'):
            comment = True
        elif char == '{':
            depth += 1
        elif char == '}':
            depth -= 1
            if depth == 0:
                return text[start:index+1]
        index += 1
    raise ValueError(f'unterminated shell function {name}')


class Harness:
    def __init__(self, root):
        self.root = root
        self.busybox = root/'bb'
        self.busybox.write_text(
            '#!/bin/sh\n'
            'cmd=$1; shift\n'
            'case "$cmd" in\n'
            '  dmesg) echo "[fake kernel] wlan0 registered";;\n'
            '  mknod) echo "MKNOD $*" >> %s/mknod.calls;;\n'
            '  *) "$cmd" "$@";;\n'
            'esac\n' % root)
        self.busybox.chmod(0o755)

    def shell(self):
        """The stage runs under busybox ash, so prefer it when present."""
        found = shutil.which('busybox')
        return [found, 'sh'] if found else ['sh']

    def run(self, body, env=None):
        script = self.root/'harness.sh'
        script.write_text(f'BB={self.busybox}\n{body}\n')
        environment = {**os.environ, **(env or {})}
        return subprocess.run(self.shell()+[str(script)], capture_output=True,
                              text=True, env=environment, timeout=30)


class WifiStartupEvidence(unittest.TestCase):
    def setUp(self):
        self.text = STAGE.read_text()
        import tempfile
        self._dir = tempfile.TemporaryDirectory()
        self.root = Path(self._dir.name)
        self.harness = Harness(self.root)
        # Keep the real logic; only relocate absolute stage paths.
        self.fail_src = extract(self.text, 'fail').replace('/tmp/', f'{self.root}/')

    def tearDown(self):
        self._dir.cleanup()

    def probe(self):
        return (self.root/'probe.log').read_text()

    def test_a_live_supplicant_without_a_socket_is_not_reported_as_an_exit(self):
        """The whole point: these two faults must not share a reason."""
        loop = self.text[self.text.index('while [ ! -S'):]
        loop = loop[:loop.index('done')]
        self.assertIn('|| fail supplicant-socket-timeout', loop)
        # A dead child is reaped first so its exit status reaches the log.
        dead = loop[loop.index('if ! $BB kill -0'):loop.index('fi', loop.index('if ! $BB kill -0'))]
        self.assertIn('wait "$supplicant"', dead)
        self.assertIn('fail supplicant-exit', dead)
        self.assertNotIn('supplicant-socket-timeout', dead)
        timeout = [line for line in loop.splitlines() if '-lt "$SOCKET_WAIT"' in line][0]
        self.assertIn('supplicant-socket-timeout', timeout)
        self.assertNotIn('fail supplicant-exit', timeout)
        # The wait outlasts a slow kernel random pool in both modes.
        waits = dict(re.findall(r'\n\s*(SOCKET_WAIT)=(\d+)', self.text))
        self.assertEqual(self.text.count('SOCKET_WAIT='), 2)
        self.assertGreaterEqual(min(int(v) for v in re.findall(r'SOCKET_WAIT=(\d+)', self.text)), 60)

    def test_failure_records_that_the_supplicant_was_still_running(self):
        body = (f'{self.fail_src}\nstarting_supplicant=1\n'
                'sleep 5 >/dev/null 2>&1 & supplicant=$!\n'
                'fail supplicant-socket-timeout\n')
        self.harness.run(body)
        log = self.probe()
        self.assertIn('WiFi failed: supplicant-socket-timeout', log)
        self.assertIn('still running at failure', log)
        self.assertNotIn('no longer running at failure', log)

    def test_failure_records_that_the_supplicant_had_died(self):
        body = (f'{self.fail_src}\nstarting_supplicant=1\n'
                'sh -c "exit 0" >/dev/null 2>&1 & supplicant=$!\nwait $supplicant\n'
                'fail supplicant-exit\n')
        self.harness.run(body)
        log = self.probe()
        self.assertIn('WiFi failed: supplicant-exit', log)
        self.assertIn('no longer running at failure', log)
        self.assertNotIn('still running at failure', log)

    def test_failure_captures_startup_log_control_directory_and_kernel_tail(self):
        (self.root/'couch-wpa-startup.log').write_text(
            'Successfully initialized wpa_supplicant\n'
            'rfkill: Cannot open RFKILL control device\n')
        (self.root/'couch-wpa').mkdir()
        body = (f'{self.fail_src}\nstarting_supplicant=1\n'
                'sleep 5 >/dev/null 2>&1 & supplicant=$!\nfail supplicant-socket-timeout\n')
        self.harness.run(body)
        log = self.probe()
        self.assertIn('Successfully initialized wpa_supplicant', log)
        self.assertIn('rfkill: Cannot open RFKILL control device', log)
        self.assertIn('WiFi control directory:', log)
        self.assertIn('[fake kernel]', log)

    def test_an_absent_control_directory_is_reported_rather_than_hiding_it(self):
        body = (f'{self.fail_src}\nstarting_supplicant=1\n'
                'sleep 5 >/dev/null 2>&1 & supplicant=$!\nfail supplicant-socket-timeout\n')
        self.harness.run(body)
        self.assertIn('WiFi control directory', self.probe())

    def test_nothing_is_captured_once_credentials_become_possible(self):
        """After startup succeeds the log may carry secrets, so stay silent."""
        (self.root/'couch-wpa-startup.log').write_text('secret-bearing output\n')
        body = (f'{self.fail_src}\nstarting_supplicant=0\nsupplicant=1\n'
                'fail dhcp-exit\n')
        self.harness.run(body)
        log = self.probe()
        self.assertIn('WiFi failed: dhcp-exit', log)
        self.assertNotIn('secret-bearing output', log)
        self.assertNotIn('at failure', log)
        self.assertNotIn('[fake kernel]', log)

    def test_the_stage_still_exposes_no_block_devices_and_does_not_run_mdev(self):
        # The script explains why mdev is avoided, so judge code, not comments.
        code = '\n'.join(line for line in self.text.splitlines()
                         if not line.lstrip().startswith('#'))
        self.assertNotIn('mdev', code)
        self.assertEqual(re.findall(r'mknod[^\n]*\s-?\s*\bb\b', code), [])
        for call in re.findall(r'mknod[^\n]*', code):
            self.assertIn(' c ', call)
        self.assertNotIn('misc_node', code)
        self.assertNotIn('rfkill', self.text)

    def test_extraction_is_not_fooled_by_braces_in_quotes_or_comments(self):
        """A brace inside a message must not end the function early."""
        sample = ('noise\n'
                  'demo() {\n'
                  '    echo "a } in double quotes"\n'
                  "    echo 'a } in single quotes'\n"
                  '    # a } in a comment\n'
                  '    if true; then echo nested; fi\n'
                  '}\n'
                  'after() { echo later; }\n')
        body = extract(sample, 'demo')
        self.assertTrue(body.startswith('demo() {'))
        self.assertTrue(body.endswith('}'))
        self.assertIn('nested', body)
        self.assertNotIn('after()', body)
        with self.assertRaises(ValueError):
            extract('broken() {\n  echo "unterminated\n', 'broken')

    def test_the_startup_capture_bound_is_unchanged(self):
        """Widening the pre-credential capture is deliberately out of scope."""
        block = extract(self.text, 'fail')
        size = int(re.search(r'tail -c (\d+) [^\n]*couch-wpa-startup\.log', block).group(1))
        self.assertEqual(size, 4096)


if __name__ == '__main__':
    unittest.main()
