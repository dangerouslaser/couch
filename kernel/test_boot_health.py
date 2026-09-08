"""Exercise the initramfs health gate against complete filesystem snapshots."""
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'initramfs/boot-health.sh'


class BootHealthTests(unittest.TestCase):
    def check_snapshot(self, marker, process='couch-gui\n'):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'tmp').mkdir()
            (root / 'proc/7').mkdir(parents=True)
            (root / 'proc/uptime').write_text('120.50 250.00\n')
            if process is not None:
                (root / 'proc/7/comm').write_text(process)
            if marker is not None:
                (root / 'tmp/couch-gui.health').write_text(marker)
            return subprocess.run(['sh', str(SCRIPT), name], capture_output=True, text=True)

    def test_fresh_heartbeat_from_gui_is_healthy_without_network_state(self):
        result = self.check_snapshot('7 119\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '7 119\n')

    def test_missing_stale_future_and_malformed_heartbeats_are_rejected(self):
        for marker in [None, '7 109\n', '7 121\n', '7 x\n', '7 119 extra\n', '../7 119\n']:
            with self.subTest(marker=marker):
                self.assertNotEqual(self.check_snapshot(marker).returncode, 0)

    def test_dead_or_reused_pid_does_not_prove_boot_health(self):
        for process in [None, 'sh\n']:
            with self.subTest(process=process):
                self.assertNotEqual(self.check_snapshot('7 119\n', process).returncode, 0)
