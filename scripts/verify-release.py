"""Verify the draft's updater packages against the public key embedded in the app."""

import argparse
import base64
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import tempfile


PACKAGES = {
    "darwin-aarch64": "Prometeu_aarch64.app.tar.gz",
    "linux-x86_64": "Prometeu_x86_64.AppImage",
}


def assemble(version, repo, directory, notes):
    """Build one manifest from the final signed artifacts, after AppImage repacking."""
    directory = Path(directory)
    platforms = {}
    for platform, name in PACKAGES.items():
        entry = {
            "url": f"https://github.com/{repo}/releases/download/v{version}/{name}",
            "signature": (directory / f"{name}.sig").read_text().strip(),
        }
        platforms[platform] = entry
        suffix = "app" if platform.startswith("darwin-") else "appimage"
        platforms[f"{platform}-{suffix}"] = entry
    manifest = {
        "version": version, "notes": notes.strip(),
        "pub_date": datetime.now(timezone.utc).isoformat(), "platforms": platforms,
    }
    (directory / "latest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def verify(version, repo, directory, public_key, expected_notes):
    directory = Path(directory)
    manifest = json.loads((directory / "latest.json").read_text())
    if manifest["version"] != version:
        raise ValueError("latest.json version does not match the tag")
    notes = manifest.get("notes")
    if not expected_notes.strip() or not isinstance(notes, str) or notes.strip() != expected_notes.strip():
        raise ValueError("latest.json notes do not match the changelog section")
    platforms = manifest["platforms"]
    if not PACKAGES.keys() <= platforms.keys():
        raise ValueError("latest.json must include macOS and Linux")
    if not (directory / "Prometeu_aarch64.dmg").is_file():
        raise ValueError("Missing Prometeu_aarch64.dmg")

    # New Tauri clients may select installer-specific entries; validate those as well.
    packages = {
        **PACKAGES,
        "darwin-aarch64-app": PACKAGES["darwin-aarch64"],
        "linux-x86_64-appimage": PACKAGES["linux-x86_64"],
    }
    with tempfile.TemporaryDirectory() as temporary:
        key = Path(temporary) / "public.key"
        signature = Path(temporary) / "package.minisig"
        key.write_bytes(base64.b64decode(public_key, validate=True))
        for platform, entry in platforms.items():
            name = packages[platform]
            expected = f"https://github.com/{repo}/releases/download/v{version}/{name}"
            if entry["url"] != expected:
                raise ValueError(f"Unexpected package URL for {platform}: {entry['url']}")
            package = directory / name
            if not package.is_file():
                raise ValueError(f"Missing {name}")
            if (directory / f"{name}.sig").read_text().strip() != entry["signature"].strip():
                raise ValueError(f"Signature does not match latest.json for {platform}")
            signature.write_bytes(base64.b64decode(entry["signature"].strip(), validate=True))
            subprocess.run(
                ["minisign", "-V", "-m", str(package), "-p", str(key), "-x", str(signature)],
                check=True,
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assemble", action="store_true")
    parser.add_argument("version")
    parser.add_argument("repo")
    parser.add_argument("directory")
    args = parser.parse_args()
    config = json.loads(Path("src-tauri/tauri.conf.json").read_text())
    notes = os.environ["RELEASE_NOTES"]
    if args.assemble:
        assemble(args.version, args.repo, args.directory, notes)
    verify(args.version, args.repo, args.directory, config["plugins"]["updater"]["pubkey"], notes)
