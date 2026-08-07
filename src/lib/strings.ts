// AGENTS.md invariant 7: all user-facing strings live here. No string
// literals in JSX text positions — and, per the invariant's own
// wording, not just JSX text: attribute-position strings that reach
// the user (aria-label, tooltip content, glyphs) live here too, even
// where the narrower JSX-text lint rule (eslint.config.js) wouldn't
// itself flag them. Grows as screens are added (PLAN.md §4).
export const strings = {
  appTitle: "llama.cpp Manager",

  sidebar: {
    navLabel: "Primary",
    collapse: "Collapse sidebar",
    expand: "Expand sidebar",
    collapseGlyph: "«", // «
    expandGlyph: "»", // »
  },

  // Shared across every screen (T-005): all seven are non-functional
  // scaffolding, not seven different half-finished features, so one
  // honest note rather than seven slightly different ones.
  emptyScreenNote: "This screen is scaffolded and not yet functional.",

  screens: {
    dashboard: {
      navLabel: "Dashboard",
      initial: "D",
      title: "Dashboard",
      description: "An overview of runtime status, the active model and recent activity.",
    },
    runtime: {
      navLabel: "Runtime",
      initial: "R",
      title: "Runtime",
      description: "Start, stop and monitor the llama-server process.",
      // T-011: the health-check screen is this route's first-run view,
      // not an eighth route (docs/TASKS.md T-011) — its copy nests here
      // rather than getting a sibling top-level entry.
      healthCheck: {
        heading: "Environment health",
        refresh: "Refresh",
        loadingLabel: "Checking environment",
        errorBody: "The environment probe failed. Try refreshing.",
        remediationLabel: "How to fix:",
        statusLabels: {
          Pass: "Pass",
          Warn: "Warning",
          Fail: "Fail",
        },
        // `HealthCheck.id`'s doc comment (src/lib/types.ts, T-002/T-010)
        // enumerates exactly these five ids. An id outside this set
        // (should the backend ever add one) falls back to a formatted
        // version of the raw id rather than an empty label — see
        // `checkLabel()` in Runtime.tsx.
        checkLabels: {
          gpu_present: "GPU detected",
          driver_ok: "Driver version",
          cuda13_ok: "CUDA 13 support",
          disk_space: "Disk space",
          endpoint_bindable: "Endpoint port",
        },
      },
    },
    models: {
      navLabel: "Models",
      initial: "M",
      title: "Models",
      description: "Browse, import and manage your local models.",
    },
    modelDetail: {
      navLabel: "Model detail",
      initial: "M",
      title: "Model detail",
      description: "Settings, calibration and launch history for a single model.",
    },
    api: {
      navLabel: "API",
      initial: "A",
      title: "API",
      description: "Endpoint configuration, the API key and the request log.",
    },
    logs: {
      navLabel: "Logs",
      initial: "L",
      title: "Logs",
      description: "Diagnostics and exportable logs.",
    },
    settings: {
      navLabel: "Settings",
      initial: "S",
      title: "Settings",
      description: "Application preferences.",
    },
  },
} as const;
