

interface StatusBadgeProps {
  status: string;
}

export function StatusBadge({ status }: StatusBadgeProps) {
  switch (status) {
    case "Cleared":
      return (
        <span className="inline-flex items-center justify-center w-5 h-5 text-xs font-bold text-green-700 bg-green-100 rounded-full">
          *
        </span>
      );
    case "Pending":
      return (
        <span className="inline-flex items-center justify-center w-5 h-5 text-xs font-bold text-yellow-700 bg-yellow-100 rounded-full">
          !
        </span>
      );
    default:
      return (
        <span className="inline-flex items-center justify-center w-5 h-5 text-xs text-gray-400">
          &middot;
        </span>
      );
  }
}
