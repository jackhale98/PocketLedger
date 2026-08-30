#!/usr/bin/env python3
"""Patch the Xcode project that `tauri ios init` generates.

Usage: patch-ios-project.py <src-tauri/gen/apple>

Idempotent: every edit checks for its own marker first, so re-running (or
running against a committed project) is a no-op. After patching, the caller
must re-run `xcodegen generate --spec <dir>/project.yml`; tauri-cli only runs
xcodegen during `ios init`.

Edits:
  * Copies src-tauri/PrivacyInfo.xcprivacy next to project.yml and adds it to
    the <App>_iOS target's sources so it ships in the bundle root (App Store
    Connect rejects uploads without a privacy manifest).
  * TARGETED_DEVICE_FAMILY = 1 (iPhone only). xcodegen defaults to "1,2",
    which lists the app for iPad, where the fixed 390px phone layout would be
    reviewed and rejected. Revisit once the layout is responsive.
"""
import re
import shutil
import sys
from pathlib import Path

gen_apple = Path(sys.argv[1]).resolve()
src_tauri = Path(__file__).resolve().parents[2] / "src-tauri"
project_yml = gen_apple / "project.yml"
if not project_yml.is_file():
    sys.exit(f"error: {project_yml} not found; run `tauri ios init` first")

# 1. Privacy manifest file.
manifest_src = src_tauri / "PrivacyInfo.xcprivacy"
manifest_dst = gen_apple / "PrivacyInfo.xcprivacy"
if not manifest_dst.exists() or manifest_dst.read_bytes() != manifest_src.read_bytes():
    shutil.copyfile(manifest_src, manifest_dst)
    print(f"copied {manifest_src.name} -> {manifest_dst}")
else:
    print("PrivacyInfo.xcprivacy already in place")

text = project_yml.read_text()
original = text

# 2. Add the manifest to the iOS target sources, right after the storyboard
#    entry that the template always emits inside `targets: <App>_iOS: sources:`.
if "PrivacyInfo.xcprivacy" not in text:
    text, n = re.subn(
        r"^(?P<indent>[ \t]+)- path: LaunchScreen\.storyboard[ \t]*$",
        lambda m: f"{m.group(0)}\n{m.group('indent')}- path: PrivacyInfo.xcprivacy",
        text,
        count=1,
        flags=re.M,
    )
    if n != 1:
        sys.exit("error: could not find '- path: LaunchScreen.storyboard' in project.yml")
    print("added PrivacyInfo.xcprivacy to target sources")
else:
    print("PrivacyInfo.xcprivacy already listed in project.yml")

# 3. iPhone only. ENABLE_BITCODE sits in the iOS target's settings.base block.
if "TARGETED_DEVICE_FAMILY" not in text:
    text, n = re.subn(
        r"^(?P<indent>[ \t]+)ENABLE_BITCODE: false[ \t]*$",
        lambda m: f"{m.group(0)}\n{m.group('indent')}TARGETED_DEVICE_FAMILY: 1",
        text,
        count=1,
        flags=re.M,
    )
    if n != 1:
        sys.exit("error: could not find 'ENABLE_BITCODE: false' in project.yml")
    print("set TARGETED_DEVICE_FAMILY: 1 (iPhone only)")
else:
    print("TARGETED_DEVICE_FAMILY already set")

if text != original:
    project_yml.write_text(text)
    print(f"wrote {project_yml}")
