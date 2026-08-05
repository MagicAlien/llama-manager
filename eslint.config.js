import js from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsParser from "@typescript-eslint/parser";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";

export default [
  { ignores: ["dist/**", "src-tauri/**", "node_modules/**", "src/lib/types.ts"] }, // types.ts is generated (T-002); linting it fights the generator, not the code.
  js.configs.recommended,
  {
    // Node-side build scripts (T-002's scripts/generate-types.mjs) run
    // under Node, not the browser globals the renderer config below
    // assumes.
    files: ["scripts/**/*.{js,mjs}"],
    languageOptions: {
      globals: {
        ...globals.node,
      },
    },
  },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
        ecmaFeatures: { jsx: true },
      },
      globals: {
        ...globals.browser,
        ...globals.es2021,
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
  {
    // T-002 / docs/adr/adr-001-type-generation.md: src/lib/ipc.ts imports
    // its types from the generated src/lib/types.ts and never redeclares
    // them — an inline type here is exactly the drift the ADR exists to
    // prevent. Scoped to this one file, not the whole src/lib directory,
    // since src/lib/types.ts is itself all type declarations by design.
    files: ["src/lib/ipc.ts"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: "TSTypeAliasDeclaration",
          message:
            "src/lib/ipc.ts must import types from ./types, not declare them inline (docs/adr/adr-001-type-generation.md).",
        },
        {
          selector: "TSInterfaceDeclaration",
          message:
            "src/lib/ipc.ts must import types from ./types, not declare them inline (docs/adr/adr-001-type-generation.md).",
        },
      ],
    },
  },
];
