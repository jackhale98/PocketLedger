import { amountTone, formatAmount } from "../../utils/format";

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

  // Through the shared formatter so this honours commodity precision and the
  // hide-amounts setting, like every other amount in the app. Zero is
  // neutral: neither gain nor loss.
  return (
    <span className={`font-mono tabular-nums ${amountTone(amount)} ${className}`}>
      {formatAmount(amount, commodity ?? "")}
    </span>
  );
}
