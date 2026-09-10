from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('ha100-power-snapshot.sh')


class PowerSnapshotTests(unittest.TestCase):
    def snapshot(self, files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(value)
            before = {name: (root / name).read_bytes() for name in files}
            result = subprocess.check_output(['sh', str(SCRIPT), directory], text=True, timeout=3)
            self.assertEqual(before, {name: (root / name).read_bytes() for name in files})
            return result

    def test_only_allowlisted_fields_and_pins_are_reported(self):
        result = self.snapshot({'sys/class/power_supply/battery/capacity': '100\n',
            'sys/class/power_supply/battery/serial_number': 'PRIVATE',
            'sys/class/misc/mtgpio/pin': '  4:00111110\n 17:00111110\n 58:00001110\n',
            'tmp/couch-gui.health': '12 100\n', 'proc/12/comm': 'couch-gui\n',
            'proc/uptime': '102.7 44.8\n'})
        self.assertNotIn('PRIVATE', result)
        self.assertNotIn('gpio.4.', result)
        self.assertIn('gpio.17.mode=0 din=1 dout=1 direction=1', result)
        self.assertIn('gui.health_age_seconds=2', result)
        self.assertIn('gui.health_fresh=yes', result)

    def test_absent_stale_and_malformed_health(self):
        self.assertIn('gui.health=unavailable', self.snapshot({}))
        for stamp in ('99', '999'):
            result = self.snapshot({'tmp/couch-gui.health': f'12 {stamp}\n',
                'proc/12/comm': 'couch-gui\n', 'proc/uptime': '200.5 0\n'})
            self.assertIn('gui.health_fresh=no', result)
        self.assertIn('gui.health=invalid', self.snapshot({
            'tmp/couch-gui.health': '../escape bad\n', 'proc/uptime': '100.5 0\n'}))


if __name__ == '__main__':
    unittest.main()
