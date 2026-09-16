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

  // T-037 — what a model can actually do, one label per tag. The same four
  // appear on the models list and on the model detail screen, so they live
  // here rather than under either screen.
  capabilities: {
    thinking: "Thinking",
    mtp: "MTP",
    vision: "Vision",
    toolUse: "Tool use",
  },

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
        undeterminedFlag: "Model loading unavailable",
        undeterminedDescription: "This build doesn't declare how to register its models, so models can't be loaded with it.",
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
      ggufFilterName: "GGUF model files",
      addFolder: "Add folder",
      rescan: "Rescan",
      rescanning: "Rescanning watched folders…",
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
      browseFolder: "Browse…",
      removeFolder: "Remove",
      addFolderBtn: "Add folder",
      watchedFolders: "Watched folders",
      duplicate: "Duplicate",
      moe: "MoE",
      preload: "Preload",
      pinned: "Pinned",
      params: "params",
      shards: "shards",
      modelsCount: "models",
      details: "Details",
      badges: {
        // T-037 — `supported` and `present` are gone: they were true of
        // almost every catalogue entry and said nothing about the model.
        // Their informative cases (`warnings`, `experimental`,
        // `unsupported`, `missing`, `unreadable`) are unchanged.
        warnings: "Warnings",
        experimental: "Experimental",
        unsupported: "Unsupported",
        missing: "Missing",
        unreadable: "Unreadable",
      },
    },
    modelDetail: {
      navLabel: "Model detail",
      initial: "M",
      title: "Model detail",
      description: "Settings, calibration and launch history for a single model.",
      loading: "Loading model…",
      notFoundTitle: "Model not found",
      notFoundBody: "This model is not in the catalogue, or it has been removed.",
      backToModels: "Back to models",
      meta: {
        size: "Size",
        params: "Parameters",
        quantization: "Quantization",
        architecture: "Architecture",
        layers: "Layers",
        context: "Context",
        path: "File path",
        shards: "Shards",
        projector: "Projector",
        projectorNone: "None",
      },
      tabs: {
        launch: "Launch",
        sampling: "Sampling",
      },
      launch: {
        gpuLayers: "GPU layers",
        gpuLayersHint: "Layers offloaded to the GPU. Higher uses more VRAM.",
        ctxSize: "Context size",
        batchSize: "Batch size",
        ubatchSize: "Micro-batch size",
        threads: "CPU threads",
        flashAttn: "Flash attention",
        flashAttnOn: "On",
        flashAttnOff: "Off",
        flashAttnAuto: "Auto",
        cacheTypeK: "K cache type",
        cacheTypeV: "V cache type",
        cacheTypeF16: "f16 (default)",
        cacheTypeBF16: "bf16",
        cacheTypeQ8: "q8_0",
        cacheTypeQ4: "q4_0",
        nCpuMoe: "CPU MoE threads",
        mainGpu: "Main GPU",
        noMmap: "Disable mmap",
        mlock: "Lock in memory",
        chatTemplate: "Chat template",
        chatTemplateHint: "Leave empty to use the chat template embedded in the model (auto-detected from the GGUF).",
        extraArgs: "Extra arguments",
        extraArgsWarning: "Unvalidated — passed to the server as-is.",
        previewTitle: "Preset preview",
        previewExplain:
          "What the manager writes for this model in the preset file the server reads at start. The server command line itself is built when the server starts, with the port and file paths chosen then.",
        previewEmpty: "No active build yet — install one to see what would be written.",
        previewError: "Preset preview unavailable:",
        experimentalNote: "This model uses an experimental quantization.",
        save: "Save launch settings",
        saved: "Launch settings saved.",
        restartBanner: "Settings saved. Restart the server to apply them.",
        dirtyBanner: "Unsaved changes. The next restart uses the last saved settings.",
        projectionTitle: "VRAM projection",
        projectionLayers: "Recommended GPU layers",
        projectionVram: "Projected VRAM",
        projectionRam: "Estimated RAM",
        projectionKv: "KV cache",
        projectionGpuVram: "GPU VRAM",
        projectionProjector: "Projector",
        projectionFits: "Fits fully in VRAM",
        projectionDoesntFit: "Does not fully fit in VRAM",
        projectionNotes: "Notes",
        projectionLoading: "Projecting…",
        // T-036 — speculative decoding. The role is read from the model's
        // GGUF header (MTP heads) or from the draft companion the user
        // attaches; nothing here is inferred from a filename.
        // (T-037 removed the separate `mtpBadge`: MTP is now one of the
        // capability tags rendered by `CapabilityTags`, so the same fact is
        // not stated twice.)
        draftBadge: "Draft companion",
        speculativeTitle: "Speculative decoding",
        speculativeMtpNote:
          "This model carries MTP heads and drafts its own tokens (spec-type draft-mtp).",
        speculativeNoneNote:
          "This model has no MTP heads. Attach a draft model to draft tokens with a companion.",
        speculativeExplicitType: "Speculative type",
        draftLabel: "Draft model",
        draftNone: "None",
        draftChoose: "Choose draft model…",
        draftReplace: "Replace",
        draftRemove: "Remove",
        draftHint:
          "Optional. A small helper model that proposes tokens for this model to verify — generation gets faster, the answers stay the same. It runs together with this model and never on its own. A normal model file is refused here: only helper models of a matching type are accepted.",
        draftTuningMtpNote:
          "This model drafts with its own built-in prediction heads. These settings tune how many tokens it proposes at a time.",
        draftTuningNote:
          "These settings tune how many tokens the helper model proposes at a time.",
        draftPathLabel: "Path",
        draftArchitectureLabel: "Architecture",
        draftSizeLabel: "Size",
        draftValidationFailed: "This file cannot be used as a draft model.",
        draftTuningTitle: "Draft tuning",
        draftTuningHint: "Empty fields use the build's own defaults.",
        draftNMax: "Draft tokens (max)",
        draftNMin: "Draft tokens (min)",
        draftPMin: "Min probability",
        draftThreads: "Draft CPU threads",
        draftCacheTypeK: "Draft K cache type",
        draftCacheTypeV: "Draft V cache type",
        draftCacheDefault: "Build default",
        draftFilterName: "GGUF model",
      },
      sampling: {
        temperature: "Temperature",
        topP: "Top-p",
        topK: "Top-k",
        minP: "Min-p",
        repeatPenalty: "Repeat penalty",
        presencePenalty: "Presence penalty",
        frequencyPenalty: "Frequency penalty",
        seed: "Seed",
        helper: "Client requests override these defaults. Set them here to define the baseline the server uses when a request does not specify a value.",
        save: "Save sampling defaults",
        saved: "Sampling defaults saved.",
      },
      error: "An error occurred.",
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
