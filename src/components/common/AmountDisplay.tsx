

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

  const displayAmount = commodity
    ? `${commodity}${amount}`
    : amount;

  return (
    <span className={`font-mono tabular-nums ${colorClass} ${className}`}>
      {displayAmount}
    </span>
  );
}
