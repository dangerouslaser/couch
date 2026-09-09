#!/usr/bin/env python3
"""Validate committed catalog provenance without network or an upstream checkout."""
import gzip
import hashlib
import json
import unittest
from pathlib import Path
from update_official_catalog import inspect, REVISION

ASSETS = Path(__file__).resolve().parents[2] / 'daemon/couch-confd/assets/ir'

class CatalogTests(unittest.TestCase):
    def test_every_packed_source_matches_its_digest_and_index_counts(self):
        index = json.loads((ASSETS / 'official-index.json').read_text())
        data = (ASSETS / 'official-data.irpack').read_bytes()
        self.assertEqual(index['source']['revision'], REVISION)
        self.assertEqual(len(index['codesets']), 5433)
        cursor = 0
        ids = set()
        for record in index['codesets']:
            with self.subTest(path=record['path']):
                self.assertEqual(record['offset'], cursor)
                cursor += record['length']
                content = gzip.decompress(data[record['offset']:cursor])
                self.assertEqual(hashlib.sha256(content).hexdigest(), record['sha256'])
                blob = b'blob ' + str(len(content)).encode() + b'\0' + content
                self.assertEqual(hashlib.sha1(blob).hexdigest(), record['blob_sha1'])
                self.assertNotIn(record['id'], ids)
                ids.add(record['id'])
                count, supported = inspect(content.decode('utf-8-sig'))
                self.assertEqual((count, supported), (record['commands_count'], record['supported_commands']))
                self.assertEqual(record['license'], 'MIT')
        self.assertEqual(cursor, len(data))

    def test_import_index_rejects_invalid_source_format(self):
        for source in ('', 'Filetype: Other\nVersion: 1', 'Filetype: IR signals file\nVersion: 1\nname: a\nname: b\ntype: raw\ntype: parsed'):
            with self.assertRaises(ValueError):
                inspect(source)

    def test_cc0_subset_retains_source_content_and_license_boundary(self):
        index = json.loads((ASSETS / 'catalog.json').read_text())
        self.assertEqual(index['source']['license_boundary'], '2319685f2cbf0cd3f809609622cade14d24fb819')
        for record in index['codesets']:
            self.assertEqual(record['license'], 'CC0-1.0')
            self.assertEqual(hashlib.sha256(record['text'].encode()).hexdigest(), record['sha256'])

if __name__ == '__main__':
    unittest.main()
