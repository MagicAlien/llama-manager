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
  close: "Close",

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
      // T-024: version management UI.
      versions: {
        heading: "Installed builds",
        empty: "No runtime builds installed yet. Install one to get started.",
        activeBadge: "Active",
        installButton: "Install new build",
        activateButton: "Activate",
        removeButton: "Remove",
        releaseNotes: "Release notes",
        installingLabel: "Installing",
        activatingLabel: "Activating",
        removingLabel: "Removing",
        installSuccess: "Build installed successfully.",
        activateSuccess: "Build activated. It will be used on the next server start.",
        removeSuccess: "Build removed.",
        error: "An error occurred.",
        notStoppedReason: "Cannot activate or remove a build while the server is running. Stop the server first.",
        lastBuildReason: "Cannot remove the last remaining build.",
        undeterminedFlag: "⚠ Registration channel undetermined",
        undeterminedDescription: "This build's registration channel could not be determined from --help. T-025 will observe this empirically.",
        installPathLabel: "Install path",
        installedAtLabel: "Installed",
        backendLabel: "Backend",
        buildTagLabel: "Build tag",
        currentVersion: "Current version",
        latestVersion: "Latest available",
        upToDate: "Up to date",
        updateAvailable: "Update available",
        checkingForUpdates: "Checking for updates…",
        loadingLabel: "Loading runtime info…",
        refresh: "Refresh",
        arrow: "→",
        flagsVerified: "flags verified",
        colon: ":",
      },
    },
    models: {
      navLabel: "Models",
      initial: "M",
      title: "Models",
      description: "Browse, import and manage your local models.",
      addModel: "Add model",
      addFolder: "Add folder",
      importing: "Importing models…",
      loadingModels: "Loading models…",
      emptyIcon: "[ ]",
      emptyTitle: "No models imported yet",
      emptyBody: "Import a model file or watch a folder to get started.",
      addFirstModel: "Import your first model",
      presetChanged: "Model catalogue changed. The next server restart will use the updated preset.",
      dismiss: "Dismiss",
      remove: "Remove",
      removeTitle: "Remove model",
      removeBody: "This model will be removed from the catalogue. The model file itself will not be deleted.",
      removeRetention: "Settings and calibration for this model are retained. If you import the same file again, its previous settings will be restored.",
      cancel: "Cancel",
      confirmRemove: "Remove model",
      addFolderTitle: "Add watched folder",
      addFolderBody: "Models in this folder will be automatically discovered. New files added later will be picked up on rescan.",
      folderPathPlaceholder: "Folder path",
      addFolderBtn: "Add folder",
      watchedFolders: "Watched folders",
      duplicate: "Duplicate",
      moe: "MoE",
      preload: "Preload",
      pinned: "Pinned",
      params: "params",
      shards: "shards",
      modelsCount: "models",
      badges: {
        supported: "Supported",
        warnings: "Warnings",
        experimental: "Experimental",
        unsupported: "Unsupported",
        present: "Present",
        missing: "Missing",
        unreadable: "Unreadable",
      },
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
