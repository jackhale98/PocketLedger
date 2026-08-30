#!/usr/bin/env python3
"""Add release signing to the build.gradle.kts that `tauri android init` writes.

Usage: patch-android-gradle.py <src-tauri/gen/android/app/build.gradle.kts>

Follows https://v2.tauri.app/distribute/sign/android/ but tolerates a missing
keystore.properties: with no keystore the release build is simply unsigned
(and build-android.yml warns), so forks without secrets still build.
Idempotent: skipped when the signingConfigs block is already present.
"""
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()

if "signingConfigs" in text:
    print("release signing already configured")
    sys.exit(0)

if "import java.io.FileInputStream" not in text:
    text = text.replace(
        "import java.util.Properties\n",
        "import java.util.Properties\nimport java.io.FileInputStream\n",
        1,
    )

signing_block = '''    signingConfigs {
        create("release") {
            // Written by build-android.yml from the ANDROID_* secrets.
            // Absent locally and on forks: the release build is then unsigned.
            val keystorePropertiesFile = rootProject.file("keystore.properties")
            if (keystorePropertiesFile.exists()) {
                val keystoreProperties = Properties()
                keystoreProperties.load(FileInputStream(keystorePropertiesFile))
                keyAlias = keystoreProperties["keyAlias"] as String
                keyPassword = keystoreProperties["keyPassword"] as String
                storeFile = file(keystoreProperties["storeFile"] as String)
                storePassword = keystoreProperties["storePassword"] as String
            }
        }
    }
'''
text, n = re.subn(r"^([ \t]+)buildTypes \{", lambda m: signing_block + m.group(0), text, count=1, flags=re.M)
if n != 1:
    sys.exit("error: could not find 'buildTypes {' in build.gradle.kts")

release_line = ('            signingConfig = if (rootProject.file("keystore.properties").exists())'
                ' signingConfigs.getByName("release") else null\n')
text, n = re.subn(r'^([ \t]+)getByName\("release"\) \{[ \t]*\n', lambda m: m.group(0) + release_line, text, count=1, flags=re.M)
if n != 1:
    sys.exit("error: could not find 'getByName(\"release\") {' in build.gradle.kts")

path.write_text(text)
print(f"added release signingConfig to {path}")
