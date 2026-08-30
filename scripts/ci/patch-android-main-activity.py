#!/usr/bin/env python3
"""Keep the WebView between the system bars on Android 15+.

Usage: patch-android-main-activity.py <src-tauri/gen/android/app/src/main/java/.../MainActivity.kt>

targetSdk 35+ enforces edge-to-edge and Tauri's template calls
enableEdgeToEdge(), but Android's WebView never populates
env(safe-area-inset-*), so the page would render under the status bar and
behind the gesture bar. Applying the system-bar (+ cutout + IME) insets as
padding on the window content view keeps the page fully visible without
touching the frontend. Idempotent via the marker comment.
"""
import re
import sys
from pathlib import Path

MARKER = "// pockethledger: apply system insets"
path = Path(sys.argv[1])
text = path.read_text()

if MARKER in text:
    print("MainActivity already patched")
    sys.exit(0)

imports = [
    "import androidx.core.view.ViewCompat",
    "import androidx.core.view.WindowInsetsCompat",
]
for imp in imports:
    if imp not in text:
        text = text.replace("import android.os.Bundle\n", f"import android.os.Bundle\n{imp}\n", 1)

insets = f'''    {MARKER}
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) {{ view, windowInsets ->
      val insets = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars()
          or WindowInsetsCompat.Type.displayCutout()
          or WindowInsetsCompat.Type.ime()
      )
      view.setPadding(insets.left, insets.top, insets.right, insets.bottom)
      WindowInsetsCompat.CONSUMED
    }}
'''
text, n = re.subn(r"^([ \t]+)super\.onCreate\(savedInstanceState\)[ \t]*\n", lambda m: m.group(0) + insets, text, count=1, flags=re.M)
if n != 1:
    sys.exit("error: could not find super.onCreate(savedInstanceState) in MainActivity.kt")

path.write_text(text)
print(f"patched {path}")
