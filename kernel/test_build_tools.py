"""Host checks for the boot-image boundaries and profile selection."""
import hashlib
import struct
import tempfile
import unittest
from pathlib import Path
from pack import repack, split_dtb
from configure import main as configure, settings


def fixture():
    tree = struct.pack('>10I', 0xd00dfeed, 72, 56, 72, 40, 17, 16, 0, 0, 16)
    tree += b'\0' * 16 + struct.pack('>4I', 1, 0, 2, 9)
    kernel = b'old-kernel' + tree
    rd = b'original-ramdisk'
    header = bytearray(2048)
    header[:8] = b'ANDROID!'
    struct.pack_into('<10I', header, 8, len(kernel), 0x80008000, len(rd), 0x84000000, 0, 0x80f00000, 0x8e000000, 2048, 0, 0)
    header[64:68] = b'test'
    image = bytes(header) + kernel.ljust(2048, b'\0') + rd.ljust(2048, b'\0')
    z = bytearray(1024)
    z[36:40] = b'\x18\x28\x6f\x01'
    return image, bytes(z), tree, rd


class ImageTests(unittest.TestCase):
    def test_preserves_dtb_ramdisk_and_boot_addresses(self):
        image, z, tree, rd = fixture()
        out, hashes = repack(image, z)
        self.assertEqual(out[64:576], image[64:576])
        for at in (12, 20, 28, 32, 36):
            self.assertEqual(out[at:at+4], image[at:at+4])
        self.assertEqual(out[2048:2048+len(z)+len(tree)], z+tree)
        self.assertEqual(out[4096:4096+len(rd)], rd)
        self.assertEqual(hashes['dtb'], hashlib.sha256(tree).hexdigest())
        out2, _ = repack(out, z, b'replacement')
        self.assertEqual(out2[4096:4107], b'replacement')

    def test_rejects_truncation_and_bad_kernel(self):
        image, z, _, _ = fixture()
        for source, kernel in [(image[:4096], z), (image, b'bad'), (b'bad', z)]:
            with self.assertRaises(ValueError):
                repack(source, kernel)

    def test_ignores_false_dtb_magic_and_requires_boundary(self):
        _, _, tree, _ = fixture()
        self.assertEqual(split_dtb(b'\xd0\x0d\xfe\xed'+b'\0'*50+tree)[1], tree)
        with self.assertRaises(ValueError):
            split_dtb(tree+b'trailer')

    def test_refuses_oversized_image_before_writing(self):
        image, z, _, _ = fixture()
        with self.assertRaises(ValueError):
            repack(image, z, b'x'*(16*1024*1024))

    def test_fragment_replaces_stale_config_and_detects_dropped_setting(self):
        with tempfile.TemporaryDirectory() as d:
            base, fragment, out = [str(Path(d)/n) for n in ['base', 'fragment', 'out']]
            Path(base).write_text('CONFIG_A=y\nCONFIG_B=300\n')
            Path(fragment).write_text('# CONFIG_A is not set\nCONFIG_C=y\n')
            Path(out).write_text('CONFIG_STALE=y\n')
            configure([base, fragment, out])
            self.assertEqual(settings(out), {'CONFIG_A':'n','CONFIG_B':'300','CONFIG_C':'y'})
            configure(['--check',fragment,out])
            Path(out).write_text('CONFIG_A=y\n')
            with self.assertRaises(SystemExit):
                configure(['--check',fragment,out])


if __name__ == '__main__':
    unittest.main()
