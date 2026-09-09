from pathlib import Path
import struct
import tempfile
import unittest
from clean_stage import StageError
from prepare_ext4 import validate_geometry, validate_image


class Ext4PreparationTests(unittest.TestCase):
    def geometry(self):
        return {'schema': 1, 'partition': 'userdata', 'source': 'fixture',
                'evidence_sha256': 'a' * 64, 'offset': 1048576, 'size': 67108864}

    def test_geometry_requires_evidence_and_rejects_identifiers_and_other_partitions(self):
        self.assertEqual(validate_geometry(self.geometry())['size'], 67108864)
        for field, value in [('partition', 'boot'), ('source', 'guessed'), ('size', True),
                             ('offset', -4096), ('size', 67108865), ('evidence_sha256', 'unknown'),
                             ('storage_id', 'private')]:
            geometry = self.geometry(); geometry[field] = value
            with self.subTest(field=field), self.assertRaises(StageError):
                validate_geometry(geometry)

    def test_raw_geometry_magic_and_legacy_feature_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'userdata.ext4'
            size = self.geometry()['size']
            block = bytearray(1024)
            struct.pack_into('<I', block, 4, size // 4096)
            struct.pack_into('<I', block, 24, 2)
            block[56:58] = b'\x53\xef'
            struct.pack_into('<III', block, 92, 0x2c, 0x42, 0x73)
            with path.open('wb') as out:
                out.truncate(size); out.seek(1024); out.write(block)
            validate_image(path, size)
            for offset, bad in [(56, b'\x3a\xff'), (4, struct.pack('<I', 1)),
                                (96, struct.pack('<I', 0x80)), (100, struct.pack('<I', 0x400))]:
                changed = bytearray(block); changed[offset:offset+len(bad)] = bad
                with path.open('r+b') as out:
                    out.seek(1024); out.write(changed)
                with self.subTest(offset=offset), self.assertRaises(StageError):
                    validate_image(path, size)
            with self.assertRaises(StageError):
                validate_image(path, size + 4096)
            link = Path(directory) / 'link'; link.symlink_to(path)
            with self.assertRaises(StageError):
                validate_image(link, size)


if __name__ == '__main__':
    unittest.main()
