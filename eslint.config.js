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
      // TypeScript already checks this, more accurately: core `no-undef`
      // doesn't see ambient global types (e.g. the global `JSX`
      // namespace @types/react declares), so it false-positives on
      // code `tsc` accepts. Well-known typescript-eslint guidance, not
      // scoped to T-005 — https://typescript-eslint.io/troubleshooting/faqs/general/#i-get-errors-from-the-no-undef-rule-about-various-typescript-types-i-use-in-my-code-such-as-nodejs
      "no-undef": "off",
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
  {
    // T-005 / docs/TASKS.md T-005 acceptance criteria: "Colours,
    // spacing and type sizes come from tokens — a lint rule rejects
    // hard-coded hex values and arbitrary pixel spacing in
    // components" and "A lint rule rejects string literals in JSX
    // text positions" (AGENTS.md invariant 7).
    //
    // Scoped to the component-authoring surface only — src/components,
    // src/screens and src/App.tsx — following the same narrow-scoping
    // pattern as the src/lib/ipc.ts block above, not applied blanket.
    // In particular this does NOT cover tailwind.config.js (the
    // tokens file itself, which legitimately contains hex values and
    // numeric spacing steps) or src/lib/strings.ts (which legitimately
    // contains the user-facing string literals JSX is meant to
    // reference instead of embedding inline). Test files are excluded
    // too: they assert behaviour, not visual design, and forcing
    // fixture/expectation strings through strings.ts would fight the
    // rule's own purpose.
    files: ["src/components/**/*.tsx", "src/screens/**/*.tsx", "src/App.tsx"],
    ignores: ["**/*.test.tsx"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          // Matches a hex colour anywhere inside a string literal —
          // both a bare `"#1a1a1a"` and Tailwind arbitrary-value
          // syntax like `"bg-[#1a1a1a]"`. Colours belong in
          // tailwind.config.js's token palette and are reached via a
          // named utility class (`bg-surface`, `text-accent`, …).
          selector: "Literal[value=/#(?:[0-9a-fA-F]{3,4}){1,2}\\b/]",
          message:
            "No hard-coded hex colours in components — add the colour to the token palette in tailwind.config.js and use a named utility class (docs/TASKS.md T-005).",
        },
        {
          // Matches Tailwind's arbitrary-value bracket syntax with a
          // px unit, e.g. `"p-[13px]"`, `"mt-[3px]"`. Spacing belongs
          // to the scale declared in tailwind.config.js; a bracketed
          // pixel value bypasses it.
          selector: "Literal[value=/\\[-?\\d+(?:\\.\\d+)?px\\]/]",
          message:
            "No arbitrary pixel spacing in components — use a spacing-scale utility class from tailwind.config.js's tokens instead of a bracketed px value (docs/TASKS.md T-005).",
        },
        {
          // Matches non-whitespace JSX text content, e.g. `<p>Hello</p>`.
          // `<p>{strings.foo}</p>` is unaffected — that's an expression
          // container, not a JSXText node.
          selector: "JSXText[value=/\\S/]",
          message:
            "No string literals in JSX text positions — add the string to src/lib/strings.ts and render it as an expression (AGENTS.md invariant 7).",
        },
      ],
    },
  },
];
