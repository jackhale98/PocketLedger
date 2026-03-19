import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "../../api/commands";
import type { CsvPreviewTransaction } from "../../api/types";

type Step = "pick-files" | "preview" | "importing" | "done";

export function CsvImportFlow({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState<Step>("pick-files");
  const [csvPath, setCsvPath] = useState("");
  const [rulesPath, setRulesPath] = useState("");
  const [previewTxns, setPreviewTxns] = useState<CsvPreviewTransaction[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [warnings, setWarnings] = useState<string[]>([]);
  const [rowsProcessed, setRowsProcessed] = useState(0);
  const [importedCount, setImportedCount] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const pickCsv = async () => {
    const selected = await open({ multiple: false });
    if (selected) setCsvPath(selected as string);
  };

  const pickRules = async () => {
    const selected = await open({ multiple: false });
    if (selected) setRulesPath(selected as string);
  };

  const handlePreview = async () => {
    setError(null);
    setLoading(true);
    try {
      const result = await api.previewCsvImport(csvPath, rulesPath);
      setPreviewTxns(result.transactions);
      setWarnings(result.warnings);
      setRowsProcessed(result.rowsProcessed);
      setSelected(new Set(result.transactions.map((_, i) => i)));
      setStep("preview");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const toggleSelect = (index: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const toggleAll = () => {
    if (selected.size === previewTxns.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(previewTxns.map((_, i) => i)));
    }
  };

  const handleImport = async () => {
    setStep("importing");
    setError(null);
    try {
      const indices = Array.from(selected).sort((a, b) => a - b);
      const result = await api.importCsv(csvPath, rulesPath, indices);
      setImportedCount(result.importedCount);
      setWarnings(result.warnings);
      setStep("done");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStep("preview");
    }
  };

  const fmtAmt = (amount: string, commodity: string) => {
    const q = parseFloat(amount);
    const isSymbol = commodity.length === 1 && "$\u20AC\u00A3\u00A5\u20B9\u20BD\u20BF".includes(commodity);
    const qs = q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
    return isSymbol ? `${commodity}${qs}` : commodity ? `${qs} ${commodity}` : qs;
  };

  const fileName = (path: string) => path.split("/").pop()?.split("\\").pop() ?? path;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={() => {
            if (step === "preview") setStep("pick-files");
            else onDone();
          }}
          className="p-2 -ml-2 text-gray-600 dark:text-gray-300"
        >
          &larr;
        </button>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">Import CSV</h2>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
        {step === "pick-files" && (
          <>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              Select a CSV file from your bank and a rules file that defines how to map columns to accounts.
            </p>

            {/* CSV file picker */}
            <button
              onClick={pickCsv}
              className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-left active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px]"
            >
              <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                {csvPath ? fileName(csvPath) : "Select CSV file..."}
              </div>
              {!csvPath && <div className="text-xs text-gray-500 dark:text-gray-400">Bank statement or export</div>}
            </button>

            {/* Rules file picker */}
            <button
              onClick={pickRules}
              className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-left active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px]"
            >
              <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                {rulesPath ? fileName(rulesPath) : "Select rules file..."}
              </div>
              {!rulesPath && <div className="text-xs text-gray-500 dark:text-gray-400">.csv.rules file for column mapping</div>}
            </button>

            {error && (
              <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
                {error}
              </div>
            )}

            <button
              onClick={handlePreview}
              disabled={!csvPath || !rulesPath || loading}
              className="w-full py-3 bg-blue-600 text-white rounded-lg text-sm font-medium disabled:opacity-50"
            >
              {loading ? "Reading..." : "Preview Import"}
            </button>
          </>
        )}

        {step === "preview" && (
          <>
            <div className="flex items-center justify-between">
              <div className="text-sm text-gray-600 dark:text-gray-400">
                {previewTxns.length} transactions from {rowsProcessed} rows
              </div>
              <button onClick={toggleAll} className="text-xs text-blue-600 dark:text-blue-400 font-medium">
                {selected.size === previewTxns.length ? "Deselect All" : "Select All"}
              </button>
            </div>

            {warnings.length > 0 && (
              <div className="bg-yellow-50 dark:bg-yellow-900/20 rounded-lg px-3 py-2">
                {warnings.map((w, i) => (
                  <div key={i} className="text-xs text-yellow-700 dark:text-yellow-400">{w}</div>
                ))}
              </div>
            )}

            {error && (
              <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
                {error}
              </div>
            )}

            <div className="divide-y divide-gray-100 dark:divide-gray-800">
              {previewTxns.map((txn, i) => (
                <button
                  key={i}
                  onClick={() => toggleSelect(i)}
                  className="w-full py-2.5 flex items-start gap-3 text-left"
                >
                  <div className={`w-5 h-5 mt-0.5 rounded border-2 shrink-0 flex items-center justify-center ${
                    selected.has(i)
                      ? "border-blue-600 bg-blue-600"
                      : "border-gray-300 dark:border-gray-600"
                  }`}>
                    {selected.has(i) && <span className="text-white text-xs">&#10003;</span>}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex justify-between items-center">
                      <span className="text-sm text-gray-900 dark:text-gray-100 truncate">
                        {txn.description}
                      </span>
                      <span className={`text-sm font-mono shrink-0 ml-2 ${
                        parseFloat(txn.amount) < 0 ? "text-red-500" : "text-green-500"
                      }`}>
                        {fmtAmt(txn.amount, txn.commodity)}
                      </span>
                    </div>
                    <div className="text-xs text-gray-500 dark:text-gray-400">
                      {txn.date} &middot; {txn.account1} &rarr; {txn.account2}
                    </div>
                  </div>
                </button>
              ))}
            </div>

            <button
              onClick={handleImport}
              disabled={selected.size === 0}
              className="w-full py-3 bg-blue-600 text-white rounded-lg text-sm font-medium disabled:opacity-50"
            >
              Import {selected.size} Transaction{selected.size !== 1 ? "s" : ""}
            </button>
          </>
        )}

        {step === "importing" && (
          <div className="text-sm text-gray-500 text-center py-8">
            Importing transactions...
          </div>
        )}

        {step === "done" && (
          <div className="text-center py-8 space-y-4">
            <div className="text-4xl">&#10003;</div>
            <div className="text-lg font-semibold text-gray-900 dark:text-gray-100">
              Imported {importedCount} transaction{importedCount !== 1 ? "s" : ""}
            </div>
            {warnings.length > 0 && (
              <div className="bg-yellow-50 dark:bg-yellow-900/20 rounded-lg px-3 py-2 text-left">
                {warnings.map((w, i) => (
                  <div key={i} className="text-xs text-yellow-700 dark:text-yellow-400">{w}</div>
                ))}
              </div>
            )}
            <button
              onClick={onDone}
              className="w-full py-3 bg-blue-600 text-white rounded-lg text-sm font-medium"
            >
              Done
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
