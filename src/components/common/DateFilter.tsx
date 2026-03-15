import { useState } from "react";
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
        <div className="flex gap-2 items-center">
          <div className="flex-1">
            <input
              type="date"
              value={dateFrom}
              onChange={(e) => onChange(e.target.value, dateTo)}
              className="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <span className="text-xs text-gray-400">to</span>
          <div className="flex-1">
            <input
              type="date"
              value={dateTo}
              onChange={(e) => onChange(dateFrom, e.target.value)}
              className="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
        </div>
      )}
    </div>
  );
}
