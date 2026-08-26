import { formatAmount } from "../../utils/format";


interface AmountDisplayProps {
  amount: string | null;
  commodity: string | null;
  className?: string;
}

export function AmountDisplay({
  amount,
  commodity,
  className = "",
}: AmountDisplayProps) {
  if (!amount) {
    return <span className={`text-gray-400 ${className}`}>--</span>;
  }

  const numericValue = parseFloat(amount);
  const isNegative = numericValue < 0;
  const colorClass = isNegative ? "text-negative" : "text-positive";

  // Through the shared formatter so this honours commodity precision and the
  // hide-amounts setting, like every other amount in the app.
  const displayAmount = formatAmount(amount, commodity ?? "");

  return (
    <span className={`font-mono tabular-nums ${colorClass} ${className}`}>
      {displayAmount}
    </span>
  );
}
