

export type TabId = "transactions" | "accounts" | "reports" | "more";

interface BottomNavProps {
  activeTab: TabId;
  onTabChange: (tab: TabId) => void;
}

const tabs: { id: TabId; label: string }[] = [
  { id: "transactions", label: "Transactions" },
  { id: "accounts", label: "Accounts" },
  { id: "reports", label: "Reports" },
  { id: "more", label: "More" },
];

export function BottomNav({ activeTab, onTabChange }: BottomNavProps) {
  return (
    <nav className="flex border-t border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 pb-safe">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className={`flex-1 py-3 text-xs font-medium text-center min-h-[48px] ${
            activeTab === tab.id
              ? "text-blue-600 dark:text-blue-400 border-t-2 border-blue-600 dark:border-blue-400 -mt-px"
              : "text-gray-500 dark:text-gray-400 active:text-gray-700"
          }`}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
