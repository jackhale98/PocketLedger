import { readTextFile } from "@tauri-apps/plugin-fs";
import * as api from "../api/commands";
import type { ImportedJournal } from "../api/types";
import { fileNameOf, isAndroid } from "./platform";

/** Copy a picked journal into app storage.
 *
 *  Android's document picker hands back content:// URIs, which only the
 *  webview process can open (the fs plugin resolves them through the content
 *  resolver), so the text is read here and posted to the backend. Everywhere
 *  else the backend copies the file itself. */
export async function importPickedJournal(path: string): Promise<ImportedJournal> {
  if (isAndroid() && path.startsWith("content://")) {
    const text = await readTextFile(path);
    return api.importJournalText(fileNameOf(path), text);
  }
  return api.importJournalFile(path);
}
