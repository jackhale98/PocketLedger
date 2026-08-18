import { useState, useEffect } from "react";
import { startOfMonth, endOfMonth, subMonths, startOfYear, endOfYear, subYears, startOfQuarter, endOfQuarter, subQuarters, format } from "date-fns";

interface DateFilterProps {
  dateFrom: string;
  dateTo: string;
  onChange: (from: string, to: string) => void;
}

type Preset = {
  label: string;
  from: () => Date;
  to: () => Date;
};

const PRESETS: Preset[] = [
  { label: "This Month", from: () => startOfMonth(new Date()), to: () => new Date() },
  { label: "Last Month", from: () => startOfMonth(subMonths(new Date(), 1)), to: () => endOfMonth(subMonths(new Date(), 1)) },
  { label: "This Quarter", from: () => startOfQuarter(new Date()), to: () => new Date() },
  { label: "Last Quarter", from: () => startOfQuarter(subQuarters(new Date(), 1)), to: () => endOfQuarter(subQuarters(new Date(), 1)) },
  { label: "YTD", from: () => startOfYear(new Date()), to: () => new Date() },
  { label: "This Year", from: () => startOfYear(new Date()), to: () => endOfYear(new Date()) },
  { label: "Last Year", from: () => startOfYear(subYears(new Date(), 1)), to: () => endOfYear(subYears(new Date(), 1)) },
];

function fmt(d: Date): string {
  return format(d, "yyyy-MM-dd");
}

export function DateFilter({ dateFrom, dateTo, onChange }: DateFilterProps) {
  const [showCustom, setShowCustom] = useState(false);
  // Local draft of the custom range so an invalid range (from > to) can be
  // shown inline without emitting it to the parent.
  const [customFrom, setCustomFrom] = useState(dateFrom);
  const [customTo, setCustomTo] = useState(dateTo);

  useEffect(() => {
    setCustomFrom(dateFrom);
    setCustomTo(dateTo);
  }, [dateFrom, dateTo]);

  const rangeInvalid = Boolean(customFrom && customTo && customFrom > customTo);

  const handleCustomChange = (from: string, to: string) => {
    setCustomFrom(from);
    setCustomTo(to);
    if (from && to && from > to) return; // invalid: don't emit
    onChange(from, to);
  };

  const activePreset = PRESETS.find(
    (p) => fmt(p.from()) === dateFrom && fmt(p.to()) === dateTo
  );

  const hasFilter = dateFrom || dateTo;

  return (
    <div className="space-y-2">
      {/* Preset pills */}
      <div className="flex gap-1.5 overflow-x-auto pb-1 -mx-1 px-1">
        <button
          onClick={() => { onChange("", ""); setShowCustom(false); }}
          className={`px-3 py-1.5 text-xs font-medium rounded-full whitespace-nowrap ${
            !hasFilter
              ? "bg-blue-600 text-white"
              : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
          }`}
        >
          All Time
        </button>
        {PRESETS.map((preset) => (
          <button
            key={preset.label}
            onClick={() => {
              onChange(fmt(preset.from()), fmt(preset.to()));
              setShowCustom(false);
            }}
            className={`px-3 py-1.5 text-xs font-medium rounded-full whitespace-nowrap ${
              activePreset?.label === preset.label
                ? "bg-blue-600 text-white"
                : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
            }`}
          >
            {preset.label}
          </button>
        ))}
        <button
          onClick={() => setShowCustom(!showCustom)}
          className={`px-3 py-1.5 text-xs font-medium rounded-full whitespace-nowrap ${
            hasFilter && !activePreset
              ? "bg-blue-600 text-white"
              : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
          }`}
        >
          Custom
        </button>
      </div>

      {/* Custom date inputs */}
      {showCustom && (
        <>
          <div className="flex gap-2 items-center">
            <div className="flex-1">
              <input
                type="date"
                value={customFrom}
                onChange={(e) => handleCustomChange(e.target.value, customTo)}
                className={`w-full px-2 py-1.5 border dark:bg-gray-800 dark:text-gray-100 rounded text-xs focus:outline-none focus:ring-2 ${
                  rangeInvalid
                    ? "border-red-400 dark:border-red-600 focus:ring-red-500"
                    : "border-gray-300 dark:border-gray-600 focus:ring-blue-500"
                }`}
              />
            </div>
            <span className="text-xs text-gray-400">to</span>
            <div className="flex-1">
              <input
                type="date"
                value={customTo}
                onChange={(e) => handleCustomChange(customFrom, e.target.value)}
                className={`w-full px-2 py-1.5 border dark:bg-gray-800 dark:text-gray-100 rounded text-xs focus:outline-none focus:ring-2 ${
                  rangeInvalid
                    ? "border-red-400 dark:border-red-600 focus:ring-red-500"
                    : "border-gray-300 dark:border-gray-600 focus:ring-blue-500"
                }`}
              />
            </div>
          </div>
          {rangeInvalid && (
            <p className="text-xs text-red-600 dark:text-red-400">
              "From" date is after "To" date — range not applied
            </p>
          )}
        </>
      )}
    </div>
  );
}
