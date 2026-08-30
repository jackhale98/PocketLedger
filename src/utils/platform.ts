import { platformInfo, resolveJournalRef as resolveJournalRefCmd } from "../api/commands";
import type { PlatformInfo } from "../api/types";

let cached: Promise<PlatformInfo> | null = null;

/** Platform info, fetched once. Falls back to desktop semantics if the
 *  command fails (e.g. very old backend). */
export function getPlatformInfo(): Promise<PlatformInfo> {
  if (!cached) {
    cached = platformInfo().catch(() => ({
      isMobile: false,
      storageDir: "",
    }));
  }
  return cached;
}

/** True inside the Android webview. The backend only reports "mobile", so
 *  the OS comes from the user agent, which Android's WebView always sets. */
export function isAndroid(): boolean {
  return typeof navigator !== "undefined" && /android/i.test(navigator.userAgent);
}

/** What to persist as "last journal". On mobile only the path RELATIVE to the
 *  storage dir is stored: iOS app-container paths embed a UUID that changes on
 *  every app update, and picker paths point into a temp Inbox the OS deletes. */
export async function toPersistedJournalRef(path: string): Promise<string> {
  const info = await getPlatformInfo();
  if (!info.isMobile) return path;
  if (info.storageDir && path.startsWith(info.storageDir + "/")) {
    return path.slice(info.storageDir.length + 1);
  }
  return path.split("/").pop() ?? path;
}

/** Resolve a persisted journal ref back to an absolute path (mobile only;
 *  also migrates absolute refs saved by older app versions). */
export async function resolveJournalRef(ref: string): Promise<string> {
  const info = await getPlatformInfo();
  if (!info.isMobile) return ref;
  return resolveJournalRefCmd(ref);
}

/** File name from a path or content:// URI, for naming an imported copy. */
export function fileNameOf(pathOrUri: string): string {
  const tail = pathOrUri.split(/[/\\]/).pop() ?? pathOrUri;
  try {
    return decodeURIComponent(tail);
  } catch {
    return tail;
  }
}
