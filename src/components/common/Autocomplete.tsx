import { useState, useRef, useEffect } from "react";

interface AutocompleteProps {
  value: string;
  onChange: (value: string) => void;
  /** Fired when the value is settled — a suggestion picked or the field left
   *  — rather than on every keystroke. */
  onCommit?: (value: string) => void;
  onSuggest: (prefix: string) => Promise<string[]>;
  placeholder?: string;
  className?: string;
  inputMode?: "text" | "decimal" | "numeric";
  "aria-label"?: string;
  enterKeyHint?: React.HTMLAttributes<HTMLInputElement>["enterKeyHint"];
}

export function Autocomplete({
  value,
  onChange,
  onCommit,
  onSuggest,
  placeholder,
  className = "",
  inputMode = "text",
  "aria-label": ariaLabel,
  enterKeyHint = "next",
}: AutocompleteProps) {
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);
  const [highlightIndex, setHighlightIndex] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  // Suggestion requests can resolve out of order; only the latest counts.
  const suggestSeq = useRef(0);

  useEffect(() => {
    if (!showSuggestions) return;
    const seq = ++suggestSeq.current;
    onSuggest(value)
      .then((results) => {
        if (seq === suggestSeq.current) setSuggestions(results.slice(0, 8));
      })
      .catch(() => {
        if (seq === suggestSeq.current) setSuggestions([]);
      });
  }, [value, showSuggestions, onSuggest]);

  // A stale response must not reopen the list after it was closed.
  useEffect(() => {
    if (!showSuggestions) suggestSeq.current++;
  }, [showSuggestions]);

  const handleSelect = (suggestion: string) => {
    onChange(suggestion);
    onCommit?.(suggestion);
    setShowSuggestions(false);
    setHighlightIndex(-1);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!showSuggestions || suggestions.length === 0) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlightIndex((prev) =>
        prev < suggestions.length - 1 ? prev + 1 : 0
      );
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlightIndex((prev) =>
        prev > 0 ? prev - 1 : suggestions.length - 1
      );
    } else if (e.key === "Enter" && highlightIndex >= 0) {
      e.preventDefault();
      handleSelect(suggestions[highlightIndex]);
    } else if (e.key === "Escape") {
      setShowSuggestions(false);
    }
  };

  return (
    <div ref={containerRef} className="relative">
      <input
        ref={inputRef}
        type="text"
        inputMode={inputMode}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={() => setShowSuggestions(true)}
        onBlur={() => {
          onCommit?.(value);
          // Delay to allow click on suggestion
          setTimeout(() => setShowSuggestions(false), 200);
        }}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        aria-label={ariaLabel ?? placeholder}
        aria-autocomplete="list"
        aria-expanded={showSuggestions && suggestions.length > 0}
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        enterKeyHint={enterKeyHint}
        className={`w-full px-3 py-2 min-h-[44px] border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent ${className}`}
      />
      {showSuggestions && suggestions.length > 0 && (
        <div role="listbox" className="absolute z-20 w-full mt-1 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-lg max-h-48 overflow-y-auto overflow-x-hidden">
          {suggestions.map((suggestion, i) => (
            <button
              key={suggestion}
              role="option"
              aria-selected={i === highlightIndex}
              className={`w-full px-3 py-2.5 min-h-[44px] text-left text-sm truncate hover:bg-gray-50 dark:hover:bg-gray-700 ${
                i === highlightIndex ? "bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400" : "text-gray-900 dark:text-gray-100"
              }`}
              onMouseDown={(e) => {
                e.preventDefault();
                handleSelect(suggestion);
              }}
            >
              {suggestion}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
