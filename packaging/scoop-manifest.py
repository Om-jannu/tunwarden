#!/usr/bin/env python3
"""Writes the Scoop manifest for a release.

    packaging/scoop-manifest.py VERSION ZIP_PATH OUT_PATH

ZIP_PATH is the Windows archive dist uploads
(tunwarden-cli-x86_64-pc-windows-msvc.zip). The manifest goes into
bucket/tunwarden.json of the Scoop bucket repository; it can update itself
from later releases through `checkver` and `autoupdate`.
"""

import hashlib
import json
import sys
import zipfile

REPO = "https://github.com/Om-jannu/tunwarden"
ZIP = "tunwarden-cli-x86_64-pc-windows-msvc.zip"


def main() -> None:
    version, zip_path, out_path = sys.argv[1:4]
    with open(zip_path, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    with zipfile.ZipFile(zip_path) as z:
        exe = [n for n in z.namelist() if n.split("/")[-1] == "tunwarden.exe"]
    if len(exe) != 1:
        sys.exit(f"expected one tunwarden.exe in {zip_path}, found {exe}")
    folder = exe[0].rsplit("/", 1)[0] if "/" in exe[0] else ""

    def arch(url: str, hash_value: object) -> dict:
        entry = {"url": url, "hash": hash_value}
        if folder:
            entry["extract_dir"] = folder
        return entry

    manifest = {
        "version": version,
        "description": "Keeps SSH and command-based tunnels running, with logs and login handling",
        "homepage": REPO,
        "license": "MIT|Apache-2.0",
        "architecture": {
            "64bit": arch(f"{REPO}/releases/download/v{version}/{ZIP}", digest)
        },
        "bin": "tunwarden.exe",
        "checkver": {"github": REPO},
        "autoupdate": {
            "architecture": {
                "64bit": arch(
                    f"{REPO}/releases/download/v$version/{ZIP}",
                    {"url": "$url.sha256"},
                )
            }
        },
    }
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    main()
