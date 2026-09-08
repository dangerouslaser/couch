#!/usr/bin/env python3
"""Forward kernel builds to Ollie, without a Mac VM or a second kernel checkout."""
import os
import shlex
import subprocess
import sys

profile = sys.argv[1]
subprocess.run(['ssh', 'ollie', 'mkdir -p ~/couch-kernel/recipe/kernel'], check=True)
subprocess.run(['rsync', '-a', 'kernel/', 'ollie:couch-kernel/recipe/kernel/'], check=True)
env = [f'{key}={os.environ[key]}' for key in ['KTREE', 'KOUT', 'KIMAGE', 'JOBS'] if key in os.environ]
command = shlex.join(['env', *env, 'sh', 'couch-kernel/recipe/kernel/build.sh', profile])
raise SystemExit(subprocess.call(['ssh', 'ollie', command]))
