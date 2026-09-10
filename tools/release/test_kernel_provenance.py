import copy
import struct
import unittest
from kernel_provenance import boot_kernel, verify, sha


class KernelProvenanceTests(unittest.TestCase):
    def setUp(self):
        dtb = struct.pack('>10I', 0xd00dfeed, 72, 56, 72, 40, 17, 16, 0, 0, 16)
        dtb += b'\0' * 16 + struct.pack('>4I', 1, 0, 2, 9)
        zimage = bytearray(1024)
        zimage[36:40] = b'\x18\x28\x6f\x01'
        kernel = bytes(zimage) + dtb
        header = bytearray(2048)
        header[:8] = b'ANDROID!'
        struct.pack_into('<10I', header, 8, len(kernel), 0, 1, 0, 0, 0, 0, 2048, 0, 0)
        self.boot = bytes(header) + kernel.ljust(2048, b'\0') + b'r'
        self.pin = dict(source_commit='a' * 40, profile='normal', config_sha256='b' * 64,
                        zimage_sha256=sha(zimage), compiler_sha256='c' * 64, container='compiler')
        self.manifest = dict(source_commit=self.pin['source_commit'], profile='normal',
                             source_status='', source_diff_sha256=sha(b''),
                             compiler_sha256='c' * 64, container='compiler',
                             sha256={'.config': 'b' * 64, 'arch/arm/boot/zImage': sha(zimage)})

    def test_matching_kernel_does_not_enable_installer(self):
        self.assertIs(verify(self.boot, self.manifest, self.pin)['installable'], False)

    def test_dirty_old_diagnostic_or_wrong_toolchain_manifest_rejected(self):
        for field, value in [('source_status', ' M driver.c'), ('source_commit', 'old'),
                             ('profile', 'diagnostic'), ('compiler_sha256', 'different'),
                             ('source_diff_sha256', 'different'), ('container', 'different')]:
            manifest = copy.deepcopy(self.manifest)
            manifest[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                verify(self.boot, manifest, self.pin)
        manifest = copy.deepcopy(self.manifest)
        manifest['sha256']['.config'] = 'wrong'
        with self.assertRaises(ValueError):
            verify(self.boot, manifest, self.pin)

    def test_kernel_tampering_and_truncation_rejected(self):
        changed = bytearray(self.boot)
        changed[2100] ^= 1
        for data in (bytes(changed), self.boot[:-1], b'bad'):
            with self.assertRaises(ValueError):
                boot_kernel(data, self.pin)


if __name__ == '__main__':
    unittest.main()
