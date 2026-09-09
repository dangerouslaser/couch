"""Boot policy regressions: outages must not expose an automatic hotspot."""
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / "stage2/setup-mode.sh"


class SetupModeTests(unittest.TestCase):
    def mode(self, networks=0, no_ui=0, hotspot=0):
        return subprocess.check_output(
            ["sh", str(SCRIPT), str(networks), str(no_ui), str(hotspot)], text=True
        ).strip()

    def test_fresh_device_uses_local_wifi(self):
        self.assertEqual(self.mode(), "local")

    def test_saved_networks_stay_normal_even_without_connectivity(self):
        for count in [1, 2, 20]:
            self.assertEqual(self.mode(count), "normal")

    def test_headless_recovery_does_not_wait_for_an_unavailable_gui(self):
        self.assertEqual(self.mode(no_ui=1), "normal")

    def test_hotspot_requires_an_explicit_request(self):
        for count in [0, 1]:
            for no_ui in [0, 1]:
                self.assertEqual(self.mode(count, no_ui, 1), "recovery")


if __name__ == "__main__":
    unittest.main()
