#!/usr/bin/env python3
"""Read only local-peer notification IDs from the app-filtered test monitor."""
import re
import sys
from pathlib import Path

lines = Path(sys.argv[1]).read_text().splitlines()
ids = set()
for i, line in enumerate(lines):
    if 'member=AddNotification' not in line or i + 2 >= len(lines):
        continue
    if lines[i + 1].strip() != 'string "io.github.oddship.GnomeQuickshare"':
        continue
    match = re.fullmatch(r'\s+string "(127\.0\.0\.1:[0-9]+)"', lines[i + 2])
    if match:
        ids.add(match[1])
print('\n'.join(sorted(ids)), end='\n' if ids else '')
