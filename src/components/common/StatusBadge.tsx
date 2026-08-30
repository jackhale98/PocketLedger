interface StatusBadgeProps {
  status: string;
}

export function StatusBadge({ status }: StatusBadgeProps) {
  switch (status) {
    case "Cleared":
      return (
        <span
          className="inline-flex items-center justify-center w-5 h-5 text-xs font-bold text-green-700 dark:text-green-300 bg-green-100 dark:bg-green-900/40 rounded-full"
          aria-label="Cleared"
        >
          *
        </span>
      );
    case "Pending":
      return (
        <span
          className="inline-flex items-center justify-center w-5 h-5 text-xs font-bold text-yellow-700 dark:text-yellow-300 bg-yellow-100 dark:bg-yellow-900/40 rounded-full"
          aria-label="Pending"
        >
          !
        </span>
      );
    default:
      return (
        <span
          className="inline-flex items-center justify-center w-5 h-5 text-xs text-gray-400 dark:text-gray-500"
          aria-label="Unmarked"
        >
          &middot;
        </span>
      );
  }
}
