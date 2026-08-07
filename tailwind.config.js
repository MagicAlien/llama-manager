/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    // T-005 (PLAN.md §2.5 / docs/TASKS.md T-005): "Define the palette,
    // spacing scale and type scale as tokens in one file so the whole
    // app inherits them." This `theme` block *is* that file — every
    // Tailwind utility class used anywhere under `src/` compiles from
    // the values below, so there is exactly one place to change the
    // design. `colors` and `fontSize` below replace Tailwind's default
    // palette/scale entirely (not `extend`), so a component literally
    // cannot reach for `bg-slate-900` or `text-2xl` — only names
    // declared here exist. This is deliberate and is what backs the
    // "colours and type sizes come from tokens" acceptance criterion,
    // alongside the `eslint.config.js` rule that rejects hard-coded
    // hex values and arbitrary-bracket pixel spacing in component code.
    //
    // The palette is an original composition — dark-first, one accent
    // — not sampled from LM Studio, which is closed source and
    // explicitly off-limits as a source of assets, icons or CSS
    // (PLAN.md §2.5, docs/TASKS.md T-005).
    colors: {
      transparent: "transparent",
      current: "currentColor",
      background: "#0b0c0e",
      surface: "#131417",
      "surface-hover": "#1b1d21",
      border: "#24262b",
      foreground: "#e7e8ea",
      "muted-foreground": "#9a9ca3",
      accent: "#5e6ad2",
      "accent-hover": "#6f80e0",
      "accent-foreground": "#f5f6ff",
      destructive: "#e5484d",
      "destructive-foreground": "#fdf4f4",

      // Health-check traffic lights (docs/TASKS.md T-011). `Fail`
      // reuses `destructive` rather than duplicating red under a second
      // name. No `Warn`/`Pass` equivalent existed before this task since
      // nothing rendered `CheckStatus` until now.
      pass: "#3dd68c",
      warn: "#e5a83d",
    },

    // Explicit 4px-base spacing scale (0 – 24rem in 0.25rem steps),
    // declared in full rather than inherited silently from Tailwind's
    // built-in default, so this file is genuinely the one place the
    // scale is defined. LM Studio-style information density calls for
    // a fine base unit; components reach for these keys (`p-2`, `h-9`,
    // `w-56`, …), never an arbitrary bracket value.
    spacing: Object.fromEntries(
      Array.from({ length: 97 }, (_, step) => [step, `${step * 0.25}rem`]),
    ),

    // Type scale — five sizes, enough for a dense, single-density UI
    // (no display/hero sizes; this is a utility app, not a marketing
    // page).
    fontSize: {
      xs: ["0.75rem", { lineHeight: "1rem" }],
      sm: ["0.8125rem", { lineHeight: "1.25rem" }],
      base: ["0.875rem", { lineHeight: "1.375rem" }],
      lg: ["1rem", { lineHeight: "1.5rem" }],
      xl: ["1.25rem", { lineHeight: "1.75rem" }],
    },

    extend: {
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI",
          "system-ui",
          "sans-serif",
        ],
      },
    },
  },
  plugins: [],
};
