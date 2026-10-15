import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

const tauri = { group: ["@tauri-apps/*"], message: "Only src/engine/ipc.ts talks to Tauri." };
const appCode = (dirs) => dirs.flatMap((d) => [`**/${d}`, `**/${d}/**`, `@/${d}`, `@/${d}/**`]);
const restrict = (...patterns) => ["error", { patterns }];

export default tseslint.config(
  { ignores: ["dist", "dist-mock", "playwright-report", "test-results", "src-tauri", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: { ...globals.browser } },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "no-restricted-imports": restrict(tauri),
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
  {
    files: ["src/ui/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["engine", "shell", "dev"]),
        message: "src/ui must not import app code; it becomes @horizon-tech/explorer-ui after M1.",
      }),
    },
  },
  {
    files: ["src/engine/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["ui", "shell", "dev"]),
        message: "src/engine must not import UI code.",
      }),
    },
  },
  { files: ["src/engine/ipc.ts"], rules: { "no-restricted-imports": "off" } },
  // TEMPORARY: M0 code, removed in Task 13. src/api.ts predates the boundary rules and
  // imports @tauri-apps/* directly; Task 13 replaces it with src/engine/ipc.ts.
  { files: ["src/api.ts"], rules: { "no-restricted-imports": "off" } },
  {
    files: ["src/shell/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["dev"]),
        message: "The dev catalogue is not part of the shell.",
      }),
    },
  },
  { files: ["e2e/**", "*.config.{js,ts}"], languageOptions: { globals: { ...globals.node } } },
);
