import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import couch_install as installer
from make_demo import create


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="couch-installer-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        create(self.root)
        self.manifest = self.root / "release" / "manifest.json"
        self.release, self.bundle = installer.load_release(self.manifest)
        self.device = installer.FileDevice(self.root / "device")
        self.identity = installer.identity_record(self.root / "identity.json")
        self.backups = self.root / "originals"
        self.original = {name: self.device.hash(name) for name in self.release["partitions"]}

    def run_install(self, resume=False, confirm="SIMULATED-HA100-001"):
        with contextlib.redirect_stdout(io.StringIO()):
            installer.install_simulated(self.release, self.bundle, self.device, self.backups,
                                        self.identity, confirm, resume)

    def test_backup_precedes_every_write_and_boot_is_last(self):
        writes = []
        actual_write = self.device.write

        def recording_write(name, source):
            journal = installer.read_json(self.backups / "journal.json")
            self.assertTrue(journal["backup_complete"])
            for original in self.original:
                self.assertEqual(installer.digest(self.backups / f"{original}.img"), self.original[original])
            writes.append(name)
            actual_write(name, source)

        self.device.write = recording_write
        self.run_install()
        self.assertEqual(writes, ["recovery", "userdata", "boot"])
        for name in installer.IDENTITY_PARTITIONS:
            self.assertEqual(self.device.hash(name), self.original[name])
        self.assertEqual(self.backups.stat().st_mode & 0o777, 0o700)
        for path in self.backups.iterdir():
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertTrue(installer.read_json(self.backups / "journal.json")["complete"])

    def test_bad_target_confirmation_performs_no_backup_or_write(self):
        with self.assertRaisesRegex(installer.InstallError, "confirmation"):
            self.run_install(confirm="another-device")
        self.assertFalse(self.backups.exists())
        self.assertEqual(self.device.hash("boot"), self.original["boot"])

    def test_changed_layout_refuses_before_backup(self):
        self.device.description["partitions"]["boot"]["offset"] += 512
        with self.assertRaisesRegex(installer.InstallError, "layout"):
            self.run_install()
        self.assertFalse(self.backups.exists())

    def test_backup_mismatch_prevents_all_writes(self):
        actual_hash = self.device.hash
        self.device.hash = lambda name: "0" * 64 if name == "boot" else actual_hash(name)
        with self.assertRaisesRegex(installer.InstallError, "Backup readback"):
            self.run_install()
        self.assertEqual(actual_hash("recovery"), self.original["recovery"])
        self.assertFalse(installer.read_json(self.backups / "journal.json")["backup_complete"])

    def test_partial_write_can_resume_without_replacing_original_backups(self):
        actual_write = self.device.write

        def interrupted(name, source):
            if name == "userdata":
                with self.device.partition(name).open("r+b") as output:
                    output.write(b"partial")
                raise OSError("simulated unplug")
            actual_write(name, source)

        self.device.write = interrupted
        with self.assertRaisesRegex(OSError, "unplug"):
            self.run_install()
        self.assertEqual(self.device.hash("boot"), self.original["boot"])
        journal = installer.read_json(self.backups / "journal.json")
        self.assertEqual(journal["writes"], {"recovery": "verified", "userdata": "writing"})
        self.device.write = actual_write
        self.run_install(resume=True)
        for name in self.original:
            self.assertEqual(installer.digest(self.backups / f"{name}.img"), self.original[name])

    def test_readback_failure_stops_before_boot(self):
        actual_write = self.device.write

        def corrupt_write(name, source):
            actual_write(name, source)
            if name == "userdata":
                with self.device.partition(name).open("r+b") as output:
                    output.write(b"broken")

        self.device.write = corrupt_write
        with self.assertRaisesRegex(installer.InstallError, "Write readback"):
            self.run_install()
        self.assertEqual(self.device.hash("boot"), self.original["boot"])

    def test_resume_checks_backup_integrity(self):
        self.run_install()
        (self.backups / "nvram.img").write_bytes(b"x" * 4096)
        with self.assertRaisesRegex(installer.InstallError, "Damaged backup"):
            self.run_install(resume=True)

    def test_resume_checks_stable_device_identity(self):
        self.run_install()
        self.device.description["storage_id"] = "another-device"
        with self.assertRaisesRegex(installer.InstallError, "different release/device"):
            self.run_install(resume=True, confirm="another-device")

    def test_resume_rechecks_previously_verified_partition(self):
        self.run_install()
        self.device.partition("recovery").write_bytes(b"x" * 4096)
        with self.assertRaisesRegex(installer.InstallError, "Previously verified"):
            self.run_install(resume=True)

    def test_existing_backup_requires_explicit_resume(self):
        self.run_install()
        with self.assertRaisesRegex(installer.InstallError, "already exists"):
            self.run_install()

    def test_release_refuses_bootloader_and_identity_writes(self):
        for name in ("preloader_a", "preloader_b", "lk", "lk2", "nvram", "para"):
            value = copy.deepcopy(self.release)
            value["images"][name] = value["images"]["boot"]
            self.manifest.write_text(json.dumps(value))
            with self.subTest(name=name), self.assertRaisesRegex(installer.InstallError, "prohibited"):
                installer.load_release(self.manifest)

    def test_bad_hash_and_oversize_image_fail_preflight(self):
        path = self.bundle / "boot.img"
        path.write_bytes(b"ANDROID!" + b"x" * 4088)
        with self.assertRaisesRegex(installer.InstallError, "SHA-256"):
            installer.load_release(self.manifest)
        path.write_bytes(b"x" * 8192)
        with self.assertRaisesRegex(installer.InstallError, "exact declared size"):
            installer.load_release(self.manifest)

    def test_overlapping_partition_layout_is_rejected(self):
        value = copy.deepcopy(self.release["partitions"])
        value["boot"]["offset"] = value["recovery"]["offset"]
        with self.assertRaisesRegex(installer.InstallError, "Overlapping"):
            installer.layout(value)

    def test_image_symlink_is_rejected(self):
        path = self.bundle / "boot.img"
        path.unlink()
        path.symlink_to(self.device.partition("boot"))
        with self.assertRaisesRegex(installer.InstallError, "non-symlink"):
            installer.load_release(self.manifest)

    def test_fifteen_character_published_serial_is_preserved(self):
        value = dict(self.identity, device_id="0127A260301T0251")
        path = self.root / "identity.json"
        path.write_text(json.dumps(value))
        self.assertEqual(installer.identity_record(path)["device_id"], value["device_id"])

    def interrupt_before_userdata(self):
        actual_write = self.device.write

        def interrupted(name, source):
            if name == "userdata":
                raise OSError("simulated unplug")
            actual_write(name, source)

        self.device.write = interrupted
        with self.assertRaises(OSError):
            self.run_install()
        self.device.write = actual_write

    def test_resume_checks_unwritten_partition_before_retrying_any_write(self):
        self.interrupt_before_userdata()
        self.device.partition("boot").write_bytes(b"x" * 4096)
        with patch.object(self.device, "write") as write:
            with self.assertRaisesRegex(installer.InstallError, "Unwritten partition changed: boot"):
                self.run_install(resume=True)
            write.assert_not_called()

    def test_changed_later_image_stops_before_retrying_any_write(self):
        self.interrupt_before_userdata()
        (self.bundle / "boot.img").write_bytes(b"x" * 4096)
        with patch.object(self.device, "write") as write:
            with self.assertRaisesRegex(installer.InstallError, "image changed after preflight: boot"):
                self.run_install(resume=True)
            write.assert_not_called()

    def test_resume_rejects_changed_recorded_layout(self):
        self.run_install()
        path = self.backups / "device.json"
        value = installer.read_json(path)
        value["storage_id"] = "edited"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(installer.InstallError, "Recorded device layout changed"):
            self.run_install(resume=True)

    def test_resume_rejects_unknown_write_state(self):
        self.run_install()
        path = self.backups / "journal.json"
        value = installer.read_json(path)
        value["writes"]["boot"] = "skip"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(installer.InstallError, "Invalid journal write states"):
            self.run_install(resume=True)

    def test_plan_cli_does_not_modify_device_or_create_files(self):
        before = {path.relative_to(self.root): path.read_bytes()
                  for path in self.root.rglob("*") if path.is_file()}
        with patch("sys.argv", ["couch-install", "plan", "--manifest", str(self.manifest),
                                "--device-dir", str(self.device.root)]), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(installer.main(), 0)
        after = {path.relative_to(self.root): path.read_bytes()
                 for path in self.root.rglob("*") if path.is_file()}
        self.assertEqual(before, after)

    def test_hardware_install_cli_refuses_without_constructing_transport(self):
        with patch("sys.argv", ["couch-install", "install"]), patch.object(installer, "FileDevice") as device:
            with contextlib.redirect_stderr(io.StringIO()) as output:
                self.assertEqual(installer.main(), 1)
            self.assertIn("USB installation is not enabled", output.getvalue())
            device.assert_not_called()


if __name__ == "__main__":
    unittest.main()
