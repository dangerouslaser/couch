import subprocess
from pathlib import Path
import tempfile
import unittest
from source_policy import check


class SourcePolicyTests(unittest.TestCase):
    def test_missing_baseline_rejected_and_descendant_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], text=True).strip()
            git('init', '-q')
            git('config', 'user.name', 'Fixture')
            git('config', 'user.email', 'fixture@example.invalid')
            git('commit', '--allow-empty', '-qm', 'baseline')
            before = git('rev-parse', 'HEAD')
            git('commit', '--allow-empty', '-qm', 'board fix')
            pin = {'source_commit': git('rev-parse', 'HEAD')}
            check(directory, pin)
            git('commit', '--allow-empty', '-qm', 'development descendant')
            check(directory, pin)
            git('checkout', '-q', before)
            with self.assertRaises(ValueError):
                check(directory, pin)


if __name__ == '__main__':
    unittest.main()
