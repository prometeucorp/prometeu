"""Release regressions with local files and a fake gh; never access GitHub."""

import base64
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPTS = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("verify_release", SCRIPTS / "verify-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
ASSETS = ["Prometeu_aarch64.dmg", "latest.json"] + [
    name + suffix for name in release.PACKAGES.values() for suffix in ("", ".sig")
]


class ReleaseTests(unittest.TestCase):
    def test_assembly_uses_final_signatures_and_preserves_updater_targets(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "Prometeu_aarch64.dmg").write_text("notarized dmg")
            for platform, name in release.PACKAGES.items():
                (directory / name).write_text(f"final {platform} package")
                (directory / f"{name}.sig").write_text(base64.b64encode(platform.encode()).decode())
            notes = "\n### Fixes\n\n- Keep plugin installation busy\n"
            release.assemble("0.18.1", "prometeucorp/prometeu", directory, notes)
            manifest = json.loads((directory / "latest.json").read_text())
            self.assertEqual(set(manifest["platforms"]), {
                "darwin-aarch64", "darwin-aarch64-app", "linux-x86_64", "linux-x86_64-appimage",
            })
            with patch.object(release.subprocess, "run") as minisign:
                release.verify("0.18.1", "prometeucorp/prometeu", directory, base64.b64encode(b"key").decode(), notes)
                self.assertEqual(minisign.call_count, 4)
                # A replaced AppImage requires a rebuilt manifest, including its installer alias.
                signature = base64.b64encode(b"repacked AppImage signature").decode()
                (directory / "Prometeu_x86_64.AppImage.sig").write_text(signature)
                with self.assertRaisesRegex(ValueError, "Signature does not match"):
                    release.verify("0.18.1", "prometeucorp/prometeu", directory, base64.b64encode(b"key").decode(), notes)
                release.assemble("0.18.1", "prometeucorp/prometeu", directory, notes)
                release.verify("0.18.1", "prometeucorp/prometeu", directory, base64.b64encode(b"key").decode(), notes)
                manifest = json.loads((directory / "latest.json").read_text())
                for platform in ("linux-x86_64", "linux-x86_64-appimage"):
                    self.assertEqual(manifest["platforms"][platform]["signature"], signature)

    def test_draft_upload_requires_all_assets_and_never_overwrites_a_published_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "scripts").mkdir()
            shutil.copy(SCRIPTS / "release.sh", directory / "scripts/release.sh")
            packages = directory / "pkg"
            packages.mkdir()
            calls = directory / "calls"
            gh = directory / "gh"
            gh.write_text(f"#!{sys.executable}\n" + '''import json, os, sys
from pathlib import Path
args = sys.argv[1:]
if args[:2] == ["release", "view"]:
    if os.environ["TEST_DRAFT"] == "missing": sys.exit(1)
    # Publication can occur immediately after returning the draft state.
    print("true" if os.environ["TEST_DRAFT"] == "publish-after-check" else os.environ["TEST_DRAFT"])
else:
    assert os.environ["TEST_DRAFT"] != "publish-after-check", "Published release was modified"
    with Path(os.environ["TEST_CALLS"]).open("a") as output:
        output.write(json.dumps(args) + "\\n")
    if "--notes-file" in args:
        assert Path(args[args.index("--notes-file") + 1]).read_text() == os.environ["RELEASE_NOTES"] + "\\n"
''')
            gh.chmod(0o755)
            cases = [(set(ASSETS) - {name}, "missing", False) for name in ASSETS]
            cases += [(set(ASSETS), "false", False), (set(ASSETS), "missing", True),
                      (set(ASSETS), "true", False), (set(ASSETS), "publish-after-check", False)]
            for assets, draft, allowed in cases:
                with self.subTest(assets=assets, draft=draft):
                    calls.unlink(missing_ok=True)
                    for name in ASSETS:
                        (packages / name).unlink(missing_ok=True)
                    for name in assets:
                        (packages / name).write_text("package")
                    result = subprocess.run(
                        ["sh", str(directory / "scripts/release.sh"), "draft", "0.18.1", str(packages)],
                        env={**os.environ, "PATH": f"{directory}:{os.environ['PATH']}",
                             "TEST_DRAFT": draft, "TEST_CALLS": str(calls),
                             "RELEASE_NOTES": "### Fixes\n\n- Keep plugin installation busy"},
                        capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode == 0, allowed, result.stderr)
                    if draft in ("true", "publish-after-check"):
                        self.assertIn("draft already exists", result.stderr)
                    uploads = allowed and draft == "missing"
                    self.assertEqual(calls.exists(), uploads)
                    if uploads:
                        commands = [json.loads(line) for line in calls.read_text().splitlines()]
                        self.assertEqual(commands[0][:2], ["release", "create"])
                        self.assertIn("--draft", commands[0])
                        self.assertIn("--verify-tag", commands[0])
                        self.assertEqual(commands[1][:2], ["release", "upload"])
                        self.assertNotIn("--clobber", commands[1])
                        self.assertEqual({Path(arg).name for arg in commands[1][-6:]}, set(ASSETS))

    def test_manifest_preserves_macos_and_checks_linux_signatures(self):
        signature = base64.b64encode(b"test signature").decode()
        notes = "\n### New\n\n- Offer Linux downloads\n"
        manifest = {"version": "0.14.0", "notes": notes.strip(), "platforms": {
            platform: {
                "url": f"https://github.com/prometeucorp/prometeu/releases/download/v0.14.0/{name}",
                "signature": signature,
            } for platform, name in release.PACKAGES.items()
        }}
        manifest["platforms"]["darwin-aarch64-app"] = copy.deepcopy(manifest["platforms"]["darwin-aarch64"])
        manifest["platforms"]["linux-x86_64-appimage"] = copy.deepcopy(manifest["platforms"]["linux-x86_64"])
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in ASSETS:
                (directory / name).write_text(signature if name.endswith(".sig") else "package")

            def verify(value):
                (directory / "latest.json").write_text(json.dumps(value))
                release.verify("0.14.0", "prometeucorp/prometeu", directory, base64.b64encode(b"key").decode(), notes)

            with patch.object(release.subprocess, "run") as minisign:
                verify(manifest)
                self.assertEqual(minisign.call_count, 4)
                self.assertEqual(
                    {Path(call.args[0][3]).name for call in minisign.call_args_list},
                    set(release.PACKAGES.values()),
                )
                self.assertTrue(all(call.kwargs["check"] for call in minisign.call_args_list))

                # Tauri 2.10.1 falls back to the generic target when an installer entry is absent.
                for omitted in [("darwin-aarch64-app",), ("linux-x86_64-appimage",),
                                ("darwin-aarch64-app", "linux-x86_64-appimage")]:
                    compatible = copy.deepcopy(manifest)
                    for platform in omitted:
                        del compatible["platforms"][platform]
                    verify(compatible)

                mutations = [
                    lambda m: m.update(version="0.13.0"),
                    lambda m: m.pop("notes"),
                    lambda m: m.update(notes=""),
                    lambda m: m.update(notes="Previous version's notes"),
                    lambda m: m.update(notes=None),
                    lambda m: m["platforms"].pop("linux-x86_64"),
                    lambda m: m["platforms"]["linux-x86_64"].update(url=m["platforms"]["darwin-aarch64"]["url"]),
                    lambda m: m["platforms"]["linux-x86_64-appimage"].update(url="https://example.com/package"),
                    lambda m: m["platforms"]["darwin-aarch64"].update(signature="different"),
                ]
                for mutate in mutations:
                    invalid = copy.deepcopy(manifest)
                    mutate(invalid)
                    with self.assertRaises(ValueError):
                        verify(invalid)
                for name in ASSETS:
                    if name == "latest.json":
                        continue
                    original = (directory / name).read_text()
                    (directory / name).unlink()
                    with self.subTest(missing=name), self.assertRaises((ValueError, FileNotFoundError)):
                        verify(manifest)
                    (directory / name).write_text(original)

                minisign.side_effect = subprocess.CalledProcessError(1, "minisign")
                with self.assertRaises(subprocess.CalledProcessError):
                    verify(manifest)

    def test_publish_requires_both_platforms_and_successful_workflow(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "scripts").mkdir()
            shutil.copy(SCRIPTS / "release.sh", directory / "scripts/release.sh")
            gh = directory / "gh"
            gh.write_text(f"#!{sys.executable}\n" + '''import json, os, sys
from pathlib import Path
args = sys.argv[1:]
if args[:2] == ["release", "view"]:
    print("true" if "isDraft" in args else os.environ["TEST_ASSETS"])
elif args[:2] == ["run", "list"]:
    print(os.environ["TEST_CONCLUSION"])
elif args[:2] == ["release", "edit"]:
    Path(os.environ["TEST_PUBLISHED"]).write_text(json.dumps(args))
else:
    sys.exit("Unexpected gh call: " + repr(args))
''')
            gh.chmod(0o755)
            published = directory / "published"
            cases = [(set(ASSETS) - {name}, "success", False) for name in ASSETS]
            cases += [(set(ASSETS), "failure", False), (set(ASSETS), "success", True)]
            for assets, conclusion, allowed in cases:
                with self.subTest(assets=assets, conclusion=conclusion):
                    result = subprocess.run(
                        ["sh", str(directory / "scripts/release.sh"), "publish", "0.14.0"],
                        env={**os.environ, "PATH": f"{directory}:{os.environ['PATH']}",
                             "TEST_ASSETS": "\n".join(sorted(assets)), "TEST_CONCLUSION": conclusion,
                             "TEST_PUBLISHED": str(published)},
                        capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode == 0, allowed, result.stderr)
                    self.assertEqual(published.exists(), allowed)
            self.assertEqual(json.loads(published.read_text()), [
                "release", "edit", "v0.14.0", "-R", "prometeucorp/prometeu", "--draft=false", "--latest",
            ])


if __name__ == "__main__":
    unittest.main()
