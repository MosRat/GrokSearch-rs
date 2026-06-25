from __future__ import annotations

import os
import stat
import subprocess
import sys
from pathlib import Path


def _binary_path() -> Path:
    name = "grok-search-rs.exe" if os.name == "nt" else "grok-search-rs"
    return Path(__file__).resolve().parent / "bin" / name


def main() -> int:
    binary = _binary_path()
    if not binary.exists():
        sys.stderr.write(
            f"grok-search-rs: packaged binary is missing: {binary}\n"
            "Reinstall the platform wheel for your operating system and CPU.\n"
        )
        return 127

    if os.name != "nt":
        mode = binary.stat().st_mode
        binary.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        os.execv(str(binary), [str(binary), *sys.argv[1:]])
        return 127

    completed = subprocess.run([str(binary), *sys.argv[1:]], check=False)
    return completed.returncode
