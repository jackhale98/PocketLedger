import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  { ignores: ["dist/**", "node_modules/**", "src-tauri/**", "target/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // Non-null assertions are used sparingly for values the surrounding
      // branch has already checked.
      "@typescript-eslint/no-non-null-assertion": "off",
      // These two arrived with react-hooks v7 for React Compiler readiness.
      // The app is not compiled, and it loads data in effects (setState
      // after an await, guarded by a sequence counter) and keeps the latest
      // callback in a ref -- both are deliberate and reviewed patterns here.
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/refs": "off",
    },
  },
  {
    files: ["src/**/*.test.{ts,tsx}"],
    languageOptions: { globals: { ...globals.node } },
  }
);
