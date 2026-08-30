import { useEffect, useState } from "react";

function isDark(): boolean {
  return typeof document !== "undefined" && document.documentElement.classList.contains("dark");
}

/** Whether the `dark` class is on the root element, kept current as the
 *  theme setting or the OS flips it. Charts draw with literal colours, so
 *  they can't lean on Tailwind's dark: variants like the rest of the UI. */
export function useDarkMode(): boolean {
  const [dark, setDark] = useState(isDark);
  useEffect(() => {
    const observer = new MutationObserver(() => setDark(isDark()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    setDark(isDark());
    return () => observer.disconnect();
  }, []);
  return dark;
}

export interface ChartTheme {
  grid: string;
  tick: { fontSize: number; fill: string };
  axisLine: string;
  tooltip: React.CSSProperties;
  zeroLine: string;
}

const DARK: ChartTheme = {
  grid: "#4b5563",
  tick: { fontSize: 10, fill: "#9ca3af" },
  axisLine: "#6b7280",
  tooltip: { backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" },
  zeroLine: "#6b7280",
};

const LIGHT: ChartTheme = {
  grid: "#e5e7eb",
  tick: { fontSize: 10, fill: "#6b7280" },
  axisLine: "#9ca3af",
  tooltip: { backgroundColor: "#ffffff", border: "1px solid #e5e7eb", borderRadius: 8, color: "#111827" },
  zeroLine: "#9ca3af",
};

/** Chart colours for the current theme. */
export function useChartTheme(): ChartTheme {
  return useDarkMode() ? DARK : LIGHT;
}
