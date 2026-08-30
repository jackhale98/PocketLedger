#!/usr/bin/env bash
# Set the app version everywhere it is duplicated, in one go.
#
#   scripts/bump-version.sh 0.2.21          # edit files only
#   scripts/bump-version.sh 0.2.21 --tag    # ...and commit + tag v0.2.21
#
# Touches package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json and
# Cargo.lock. The three release workflows trigger on the v* tag.
set -euo pipefail

VERSION="${1:-}"
TAG="${2:-}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <major.minor.patch> [--tag]" >&2
  exit 1
fi
if [[ -n "$TAG" && "$TAG" != "--tag" ]]; then
  echo "unknown option: $TAG" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

# package.json: only the top-level "version" key.
node -e '
  const fs = require("fs");
  const p = "package.json";
  const s = fs.readFileSync(p, "utf8");
  const out = s.replace(/^(\s*"version":\s*")[^"]*(")/m, `$1${process.argv[1]}$2`);
  if (out === s) { console.error("package.json: version key not found"); process.exit(1); }
  fs.writeFileSync(p, out);
' "$VERSION"

# tauri.conf.json: the top-level "version" is the first one in the file.
python3 - "$VERSION" <<'PY'
import re, sys
v = sys.argv[1]
p = "src-tauri/tauri.conf.json"
s = open(p).read()
out, n = re.subn(r'^(\s*"version":\s*")[^"]*(")', lambda m: m.group(1) + v + m.group(2), s, count=1, flags=re.M)
assert n == 1, "tauri.conf.json: version key not found"
open(p, "w").write(out)
PY

# Cargo.toml: the [package] version is the first `version = ` line.
python3 - "$VERSION" <<'PY'
import re, sys
v = sys.argv[1]
p = "src-tauri/Cargo.toml"
s = open(p).read()
out, n = re.subn(r'^version = "[^"]*"', f'version = "{v}"', s, count=1, flags=re.M)
assert n == 1, "Cargo.toml: version key not found"
open(p, "w").write(out)
PY

# Refresh Cargo.lock without touching the network.
cargo update -p pockethledger --offline --quiet

git --no-pager diff --stat -- package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json Cargo.lock
echo "Version set to $VERSION"

if [[ "$TAG" == "--tag" ]]; then
  git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json Cargo.lock
  git commit -m "Release $VERSION"
  git tag -a "v$VERSION" -m "PocketHLedger $VERSION"
  echo "Tagged v$VERSION. Push with: git push origin main v$VERSION"
fi
