"""Locate the F1 Manager 2024 install without assuming a drive letter.

Tools here used to hardcode one machine's path, which is wrong for anyone on a different drive and
useless on a second PC. This resolves it in order of increasing guesswork:

1. an explicit path passed in (`--game`)
2. `COOP_GAME_DIR`
3. Steam's registered library folders, from the registry and `steamapps/libraryfolders.vdf`, so any
   drive is found
4. the usual default locations, as a last resort

The game is identified by `F1Manager24.exe` under `F1Manager24/Binaries/Win64`, so a wrong match is
unlikely and every candidate is verified before it is returned.
"""

from __future__ import annotations

import os
import re
import sys

GAME_NAME = 'F1 Manager 2024'
GAME_EXE_REL = os.path.join('F1Manager24', 'Binaries', 'Win64', 'F1Manager24.exe')


def is_game_dir(path):
    """True when `path` is an install root (the folder holding `F1Manager24/`)."""
    if not path:
        return False
    return os.path.isfile(os.path.join(path, GAME_EXE_REL))


def _steam_roots_from_registry():
    """Steam's own install path, per the registry. Windows only; quiet elsewhere."""
    if os.name != 'nt':
        return []
    try:
        import winreg
    except ImportError:
        return []
    out = []
    for hive, key in (
        (winreg.HKEY_CURRENT_USER, r'Software\Valve\Steam'),
        (winreg.HKEY_LOCAL_MACHINE, r'SOFTWARE\WOW6432Node\Valve\Steam'),
        (winreg.HKEY_LOCAL_MACHINE, r'SOFTWARE\Valve\Steam'),
    ):
        try:
            with winreg.OpenKey(hive, key) as k:
                for value in ('SteamPath', 'InstallPath'):
                    try:
                        p = winreg.QueryValueEx(k, value)[0]
                    except OSError:
                        continue
                    if p and os.path.isdir(p):
                        out.append(p)
        except OSError:
            continue
    return out


def _library_folders(steam_root):
    """Every library folder Steam knows about, including the root itself."""
    roots = [steam_root]
    vdf = os.path.join(steam_root, 'steamapps', 'libraryfolders.vdf')
    try:
        with open(vdf, 'r', encoding='utf-8', errors='replace') as f:
            text = f.read()
    except OSError:
        return roots
    # The file has two shapes across Steam versions: `"1" "D:\\SteamLibrary"` and
    # `"path" "D:\\SteamLibrary"`. Match either; duplicates are removed below.
    for m in re.finditer(r'"path"\s*"([^"]+)"', text):
        roots.append(m.group(1))
    for m in re.finditer(r'^\s*"\d+"\s*"([^"]+)"\s*$', text, re.MULTILINE):
        roots.append(m.group(1))
    seen, out = set(), []
    for r in roots:
        r = r.replace('\\\\', '\\')  # the vdf escapes backslashes
        key = os.path.normcase(os.path.abspath(r))
        if key not in seen and os.path.isdir(r):
            seen.add(key)
            out.append(r)
    return out


def candidate_roots():
    """Game-install candidates, most trustworthy first."""
    out = []

    env = os.environ.get('COOP_GAME_DIR')
    if env:
        out.append(env)

    for steam in _steam_roots_from_registry():
        for lib in _library_folders(steam):
            out.append(os.path.join(lib, 'steamapps', 'common', GAME_NAME))

    # Last resort: the layouts Steam uses by default. Listed, not assumed - each is verified.
    for guess in (
        os.path.join(r'C:\Program Files (x86)\Steam\steamapps\common', GAME_NAME),
        os.path.join(r'C:\Program Files\Steam\steamapps\common', GAME_NAME),
        os.path.expanduser('~/.steam/steam/steamapps/common/' + GAME_NAME),
        os.path.expanduser('~/.local/share/Steam/steamapps/common/' + GAME_NAME),
    ):
        out.append(guess)

    seen, uniq = set(), []
    for p in out:
        key = os.path.normcase(os.path.abspath(p))
        if key not in seen:
            seen.add(key)
            uniq.append(p)
    return uniq


def find_game_dir(explicit=None):
    """The game install root, or None. Never raises."""
    if explicit:
        return explicit if is_game_dir(explicit) else None
    for cand in candidate_roots():
        if is_game_dir(cand):
            return cand
    return None


def explain_failure():
    """Print how to point at the game, and return the message."""
    lines = ['Could not find the F1 Manager 2024 install.', '',
             'Looked for %s under:' % GAME_EXE_REL]
    for t in candidate_roots():
        lines.append('  ' + t)
    lines += [
        '',
        'Set COOP_GAME_DIR to the install folder (the one containing F1Manager24):',
        '  set COOP_GAME_DIR=D:\\Games\\F1 Manager 2024        (cmd)',
        '  $env:COOP_GAME_DIR = "D:\\Games\\F1 Manager 2024"   (PowerShell)',
        'or pass --game <path>.',
    ]
    msg = '\n'.join(lines)
    print(msg, file=sys.stderr)
    return msg


if __name__ == '__main__':
    found = find_game_dir(sys.argv[1] if len(sys.argv) > 1 else None)
    if found:
        print(found)
    else:
        explain_failure()
        sys.exit(1)
