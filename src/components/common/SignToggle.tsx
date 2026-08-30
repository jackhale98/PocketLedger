import { toggleSignInput } from "../../utils/amount";

/** iOS's decimal keypad has no minus key, so amount fields get a ± button
 *  that flips the sign of whatever has been typed. */
export function SignToggle({
  value,
  onChange,
  className = "",
}: {
  value: string;
  onChange: (next: string) => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(toggleSignInput(value))}
      aria-label="Flip sign"
      title="Flip sign"
      className={`shrink-0 w-11 min-h-[44px] rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-700 dark:text-gray-200 text-base font-medium active:bg-gray-100 dark:active:bg-gray-600 ${className}`}
    >
      &plusmn;
    </button>
  );
}
