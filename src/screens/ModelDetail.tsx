import { useEffect, useMemo, useState } from "react";
import {
  estimateVram,
  getServerState,
  getModel,
  listModels,
  previewPresetParams,
  updateModelParams,
  validateDraftCompanion,
} from "@/lib/ipc";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { strings } from "@/lib/strings";
import type {
  DraftCompanionInfo,
  LaunchParams,
  ModelEntry,
  SamplingDefaults,
  ServerState,
  SpeculativeParams,
  VramEstimate,
} from "@/lib/types";
import { describeError } from "@/lib/errors";
import { getModelIdFromHash } from "@/lib/useHashRoute";
import { Loading } from "@/components/ui/loading";
import { Badge } from "@/components/ui/badge";
import { CapabilityTags } from "@/components/CapabilityTags";

// T-035: the model-detail screen. Launch and Sampling tabs, layer-budget
// slider with live projection, live preset preview (same code path as T-033)
// and a dirty-config banner.
//
// The screen is reached at `#/models/detail/<id>` (from the Models list)
// or `#/models/detail` (the sidebar link), which shows a picker.

type TabId = "launch" | "sampling";

// The fully-unset launch configuration: every field null, no extra args.
// LaunchParams is a non-optional-shape (Option<T> → T | null), so an
// "empty" value must still name every field.
const EMPTY_LAUNCH_PARAMS: LaunchParams = {
  gpu_layers: null,
  ctx_size: null,
  batch_size: null,
  ubatch_size: null,
  flash_attn: null,
  cache_type_k: null,
  cache_type_v: null,
  n_cpu_moe: null,
  tensor_split: null,
  main_gpu: null,
  no_mmap: null,
  mlock: null,
  threads: null,
  chat_template: null,
  mmproj_path: null,
  speculative: null,
  extra_args: [],
};

// T-036 — an empty speculative configuration. A null `draft_companion` means
// the model drafts with its own MTP heads; the tuning fields start empty,
// which means "use the build's own defaults", not "zero".
function emptySpeculative(companion: string | null): SpeculativeParams {
  return {
    draft_companion: companion,
    n_max: null,
    n_min: null,
    p_min: null,
    threads: null,
    cache_type_k: null,
    cache_type_v: null,
  };
}

const EMPTY_SAMPLING_DEFAULTS: SamplingDefaults = {
  temperature: null,
  top_p: null,
  top_k: null,
  min_p: null,
  repeat_penalty: null,
  presence_penalty: null,
  frequency_penalty: null,
  seed: null,
};

function formatBytes(bytes: bigint | number): string {
  const b = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (b >= 1024 * 1024 * 1024) return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (b >= 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(0)} MB`;
  return `${(b / 1024).toFixed(0)} KB`;
}

function formatParamCount(count: bigint | number | null): string {
  if (count === null) return "?";
  const c = typeof count === "bigint" ? Number(count) : count;
  if (c >= 1_000_000_000) {
    const b = c / 1_000_000_000;
    return `${Number.isInteger(b) ? b : b.toFixed(1)}B`;
  }
  if (c >= 1_000_000) {
    const m = c / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`;
  }
  return `${c}`;
}

// A numeric launch/sampling field. Empty input = null (the field is unset).
function numberField(
  label: string,
  value: number | null,
  onChange: (value: number | null) => void,
  step?: number,
) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <input
        type="number"
        step={step}
        className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))}
      />
    </label>
  );
}

// A free-text launch field (e.g. cache type). Empty input = null.
function textField(label: string, value: string | null, onChange: (value: string | null) => void) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <input
        type="text"
        className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      />
    </label>
  );
}

// A dropdown launch field. Only values the backend can actually use appear
// here — an option that cannot be chosen is a bug, not a default.
function selectField(
  label: string,
  value: string,
  options: ReadonlyArray<{ value: string; label: string }>,
  onChange: (value: string) => void,
) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <select
        className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

// The K/V cache types the build accepts. `cacheTypeDefault` is the unset
// state — the build's own default stands, which is not the same as picking
// one of the four.
const CACHE_TYPE_OPTIONS = [
  { value: "", label: strings.screens.modelDetail.launch.draftCacheDefault },
  { value: "f16", label: strings.screens.modelDetail.launch.cacheTypeF16 },
  { value: "bf16", label: strings.screens.modelDetail.launch.cacheTypeBF16 },
  { value: "q8_0", label: strings.screens.modelDetail.launch.cacheTypeQ8 },
  { value: "q4_0", label: strings.screens.modelDetail.launch.cacheTypeQ4 },
];

export function ModelDetailScreen() {
  const [modelId, setModelId] = useState<string | null>(() =>
    getModelIdFromHash(window.location.hash),
  );
  const [pickerModels, setPickerModels] = useState<ModelEntry[] | null>(null);
  const [model, setModel] = useState<ModelEntry | null | undefined>(undefined);
  const [tab, setTab] = useState<TabId>("launch");
  const [draftLaunch, setDraftLaunch] = useState<LaunchParams>(EMPTY_LAUNCH_PARAMS);
  const [draftSampling, setDraftSampling] = useState<SamplingDefaults>(EMPTY_SAMPLING_DEFAULTS);
  const [savedLaunch, setSavedLaunch] = useState<LaunchParams>(EMPTY_LAUNCH_PARAMS);
  const [savedSampling, setSavedSampling] = useState<SamplingDefaults>(EMPTY_SAMPLING_DEFAULTS);
  const [projection, setProjection] = useState<VramEstimate | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [serverState, setServerState] = useState<ServerState | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // T-036: the validated facts about the selected draft companion, and the
  // reason a candidate was refused. Validation is the backend's call — the
  // picker never decides on its own that a file is usable.
  const [companionInfo, setCompanionInfo] = useState<DraftCompanionInfo | null>(null);
  const [companionError, setCompanionError] = useState<string | null>(null);
  const [pickingDraft, setPickingDraft] = useState(false);

  const speculative = draftLaunch.speculative;
  const draftCompanionPath = speculative?.draft_companion ?? null;

  // Re-read the id when the hash changes (picker navigation, back button).
  useEffect(() => {
    const onHashChange = () => setModelId(getModelIdFromHash(window.location.hash));
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  // With an id: load the model and seed the drafts from its stored params
  // in the same callback (not a second effect). Without: load the picker.
  // Uses the repo's data-loading convention (async fn + try/finally) so the
  // react-hooks/set-state-in-effect rule stays green, matching Models.tsx.
  useEffect(() => {
    if (modelId) {
      const loadModel = async () => {
        let m: ModelEntry | null = null;
        try {
          m = await getModel(modelId);
        } finally {
          setModel(m);
          if (m) {
            // Config is auto-generated at import time (T-035 prefill),
            // so saved params are always present — no empty forms.
            setDraftLaunch(m.launch_params);
            setSavedLaunch(m.launch_params);
            setDraftSampling(m.sampling_defaults);
            setSavedSampling(m.sampling_defaults);
            setJustSaved(false);
            setSaveError(null);
          }
        }
      };
      loadModel();
    } else {
      const loadPicker = async () => {
        let models: ModelEntry[] = [];
        try {
          models = await listModels();
        } finally {
          setPickerModels(models);
        }
      };
      loadPicker();
    }
  }, [modelId]);

  const setLaunch = (params: LaunchParams) => {
    setDraftLaunch(params);
    setJustSaved(false);
  };

  // Live VRAM projection: debounced so slider drags do not fire an IPC
  // call per tick; the call itself is async and never blocks rendering.
  // Runs on all tabs — the projection card sits above the tab content.
  useEffect(() => {
    if (!model) return;
    const timer = setTimeout(() => {
      estimateVram(model.id, draftLaunch)
        .then(setProjection)
        .catch(() => setProjection(null));
    }, 150);
    return () => clearTimeout(timer);
  }, [model, draftLaunch]);

  // Live command preview: same T-033 code path (preview_preset_params),
  // debounced a little longer than the projection.
  useEffect(() => {
    if (!model || tab !== "launch") return;
    const timer = setTimeout(() => {
      previewPresetParams(model.id, draftLaunch)
        .then((text) => {
          setPreview(text);
          setPreviewError(null);
        })
        .catch((err: unknown) => {
          setPreview(null);
          setPreviewError(describeError(err));
        });
    }, 400);
    return () => clearTimeout(timer);
  }, [model, draftLaunch, tab]);

  const serverRunning = serverState !== null && typeof serverState === "object" && "Running" in serverState;

  // T-036: keep the companion's validated facts in sync with the form — both
  // for a companion just picked and for one restored from the stored params.
  // A file deleted after it was saved surfaces here as a refused candidate
  // rather than as a silent preset pointing at nothing.
  useEffect(() => {
    const timer = setTimeout(() => {
      if (draftCompanionPath === null) {
        setCompanionInfo(null);
        setCompanionError(null);
        return;
      }
      validateDraftCompanion(draftCompanionPath)
        .then((info) => {
          setCompanionInfo(info);
          setCompanionError(null);
        })
        .catch((err: unknown) => {
          setCompanionInfo(null);
          setCompanionError(describeError(err));
        });
    }, 150);
    return () => clearTimeout(timer);
  }, [draftCompanionPath]);

  // Pick a draft companion. The OS dialog can filter by extension only, so
  // the architecture check is the backend's (`validate_draft_companion`): a
  // complete model chosen here is refused, with the reason shown.
  const chooseDraftCompanion = async () => {
    if (!model) return;
    setPickingDraft(true);
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [
          {
            name: strings.screens.modelDetail.launch.draftFilterName,
            extensions: ["gguf"],
          },
        ],
      });
      if (typeof selected !== "string") return;
      const info = await validateDraftCompanion(selected);
      setCompanionInfo(info);
      setCompanionError(null);
      setLaunch({
        ...draftLaunch,
        speculative: { ...(speculative ?? emptySpeculative(null)), draft_companion: selected },
      });
    } catch (err: unknown) {
      setCompanionInfo(null);
      setCompanionError(describeError(err));
    } finally {
      setPickingDraft(false);
    }
  };

  // Patch one draft-stage field. The object is created on first edit, so a
  // model that drafts with its own MTP heads can be tuned without ever
  // picking a companion file.
  const updateSpeculative = (patch: Partial<SpeculativeParams>) => {
    setLaunch({
      ...draftLaunch,
      speculative: { ...(speculative ?? emptySpeculative(null)), ...patch },
    });
  };

  // Dropping the companion returns the model to its own head configuration:
  // an MTP model drafts with its MTP heads again, an ordinary one does not
  // draft at all.
  const removeDraftCompanion = () => {
    setCompanionInfo(null);
    setCompanionError(null);
    // The draft-stage tuning survives: an MTP model still drafts with its own
    // heads after its companion is dropped.
    setLaunch({
      ...draftLaunch,
      speculative: speculative ? { ...speculative, draft_companion: null } : null,
    });
  };

  const isDirty = useMemo(
    () =>
      JSON.stringify(draftLaunch) !== JSON.stringify(savedLaunch) ||
      JSON.stringify(draftSampling) !== JSON.stringify(savedSampling),
    [draftLaunch, savedLaunch, draftSampling, savedSampling],
  );

  const handleSave = async () => {
    if (!model) return;
    setSaving(true);
    setSaveError(null);
    try {
      const updated = await updateModelParams(model.id, draftLaunch, draftSampling);
      setSavedLaunch(updated.launch_params);
      setSavedSampling(updated.sampling_defaults);
      setJustSaved(true);
      setServerState(await getServerState());
    } catch (err: unknown) {
      setSaveError(describeError(err));
    } finally {
      setSaving(false);
    }
  };

  // ── No id: the picker ──────────────────────────────────────────
  if (!modelId) {
    return (
      <section className="flex flex-col gap-4">
        <h1 className="text-xl font-semibold text-foreground">
          {strings.screens.modelDetail.title}
        </h1>
        {pickerModels === null ? (
          <Loading label={strings.screens.modelDetail.loading} />
        ) : pickerModels.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {strings.screens.modelDetail.notFoundBody}
          </p>
        ) : (
          <div className="divide-y divide-border rounded-lg border border-border bg-surface">
            {pickerModels.map((m) => (
              <a
                key={m.id}
                href={`#/models/detail/${m.id}`}
                className="flex items-center justify-between px-4 py-3 text-sm text-foreground hover:bg-surface-hover"
              >
                <span>{m.display_name}</span>
                <span className="text-xs text-muted-foreground">{m.metadata.quantization}</span>
              </a>
            ))}
          </div>
        )}
      </section>
    );
  }

  // ── Loading / not found ────────────────────────────────────────
  if (model === undefined) {
    return (
      <section className="flex flex-col gap-4">
        <h1 className="text-xl font-semibold text-foreground">
          {strings.screens.modelDetail.title}
        </h1>
        <Loading label={strings.screens.modelDetail.loading} />
      </section>
    );
  }

  if (model === null) {
    return (
      <section className="flex flex-col gap-4">
        <h1 className="text-xl font-semibold text-foreground">
          {strings.screens.modelDetail.title}
        </h1>
        <p className="text-sm text-foreground">{strings.screens.modelDetail.notFoundTitle}</p>
        <p className="text-sm text-muted-foreground">{strings.screens.modelDetail.notFoundBody}</p>
        <a
          href="#/models"
          className="text-sm text-accent hover:text-accent-hover"
        >
          {strings.screens.modelDetail.backToModels}
        </a>
      </section>
    );
  }

  // ── The detail view ────────────────────────────────────────────
  const experimentalNote =
    model.compatibility !== "Supported" && "SupportedWithWarnings" in model.compatibility
      ? model.compatibility.SupportedWithWarnings.find((n) => n.experimental)
      : undefined;

  const meta = [
    { label: strings.screens.modelDetail.meta.size, value: formatBytes(model.size_bytes) },
    {
      label: strings.screens.modelDetail.meta.params,
      value: formatParamCount(model.metadata.param_count),
    },
    { label: strings.screens.modelDetail.meta.quantization, value: model.metadata.quantization },
    { label: strings.screens.modelDetail.meta.architecture, value: model.metadata.architecture },
    { label: strings.screens.modelDetail.meta.layers, value: `${model.metadata.block_count}` },
    // Context length is shown only when the GGUF declares it: a missing
    // value is common, and a fixed "?" adds no information.
    ...(model.metadata.context_length !== null
      ? [
          {
            label: strings.screens.modelDetail.meta.context,
            value: `${model.metadata.context_length}`,
          },
        ]
      : []),
  ];

  return (
    <section className="flex flex-col gap-4">
      <header>
        <a href="#/models" className="text-xs text-accent hover:text-accent-hover">
          {strings.screens.modelDetail.backToModels}
        </a>
        {/* T-037 — one full-width row of capability tags, above the model's
            own text. A model that declares none renders no row at all. The
            MTP badge that used to sit beside the name is now one of these
            tags, so the same fact is not stated twice. */}
        <div className="mt-2">
          <CapabilityTags tags={model.capability_tags} />
        </div>
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <h1 className="text-xl font-semibold text-foreground">{model.display_name}</h1>
          {/* T-036: the role comes from the header, never from the filename. */}
          {draftCompanionPath !== null && (
            <Badge variant="info">{strings.screens.modelDetail.launch.draftBadge}</Badge>
          )}
        </div>
        <p className="text-xs text-muted-foreground break-all">{model.file_path}</p>
      </header>

      <div className="flex flex-wrap gap-x-6 gap-y-2 rounded-lg border border-border bg-surface p-4">
        {meta.map((item) => (
          <div key={item.label} className="flex flex-col">
            <dt className="text-xs text-muted-foreground">{item.label}</dt>
            <dd className="text-sm text-foreground">{item.value}</dd>
          </div>
        ))}
        {model.shard_paths.length > 0 && (
          <div className="flex flex-col">
            <dt className="text-xs text-muted-foreground">
              {strings.screens.modelDetail.meta.shards}
            </dt>
            <dd className="text-sm text-foreground">{`${model.shard_paths.length + 1}`}</dd>
          </div>
        )}
        {/* The projector is discovered by naming convention at import time
            (PROGRESS.md F-000) — shown as resolved, never as a free field. */}
        <div className="flex flex-col">
          <dt className="text-xs text-muted-foreground">
            {strings.screens.modelDetail.meta.projector}
          </dt>
          <dd className="text-xs text-foreground break-all">
            {draftLaunch.mmproj_path ?? strings.screens.modelDetail.meta.projectorNone}
          </dd>
        </div>
      </div>

      {/* Live VRAM projection: sits right under the model details — the
          numbers are about THIS model on THIS machine, so they belong
          next to its identity, not buried at the bottom of a tab. */}
      <div className="flex flex-col gap-2 rounded-lg border border-border bg-surface p-4">
        <span className="text-xs font-semibold text-foreground">
          {strings.screens.modelDetail.launch.projectionTitle}
        </span>
        {projection ? (
          <div className="flex flex-col gap-2">
            <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm text-foreground">
              <span>
                {`${strings.screens.modelDetail.launch.projectionLayers}: ${projection.recommended_gpu_layers}`}
              </span>
              <span>
                {`${strings.screens.modelDetail.launch.projectionVram}: ${formatBytes(projection.estimated_vram_bytes)}`}
              </span>
              <span>
                {`${strings.screens.modelDetail.launch.projectionRam}: ${formatBytes(projection.estimated_ram_bytes)}`}
              </span>
              <span>
                {`${strings.screens.modelDetail.launch.projectionKv}: ${formatBytes(projection.kv_cache_bytes)}`}
              </span>
              <span>
                {`${strings.screens.modelDetail.launch.projectionGpuVram}: ${formatBytes(projection.vram_free_bytes)} / ${formatBytes(projection.vram_total_bytes)}`}
              </span>
              {projection.projector_bytes > 0 && (
                <span>
                  {`${strings.screens.modelDetail.launch.projectionProjector}: ${formatBytes(projection.projector_bytes)}`}
                </span>
              )}
              {/* T-038 — the terms the estimate accounts for beyond weights and
                  KV. Each is rendered only when it is non-zero: a row that is
                  true of almost every model carries no information, and a fixed
                  placeholder for a term that does not apply reads as a bug. */}
              {projection.draft_bytes > 0 && (
                <span>
                  {`${strings.screens.modelDetail.launch.projectionDraft}: ${formatBytes(projection.draft_bytes)}`}
                </span>
              )}
              {projection.recurrent_state_bytes > 0 && (
                <span>
                  {`${strings.screens.modelDetail.launch.projectionRecurrent}: ${formatBytes(projection.recurrent_state_bytes)}`}
                </span>
              )}
              {projection.mtp_draft_bytes > 0 && (
                <span>
                  {`${strings.screens.modelDetail.launch.projectionMtp}: ${formatBytes(projection.mtp_draft_bytes)}`}
                </span>
              )}
            </div>
            <span
              className={`text-xs ${projection.fits_fully ? "text-pass" : "text-warn"}`}
            >
              {projection.fits_fully
                ? strings.screens.modelDetail.launch.projectionFits
                : strings.screens.modelDetail.launch.projectionDoesntFit}
            </span>
            {projection.notes.length > 0 && (
              <ul className="list-inside list-disc text-xs text-muted-foreground">
                {projection.notes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            )}
          </div>
        ) : (
          <Loading label={strings.screens.modelDetail.launch.projectionLoading} size="sm" />
        )}
      </div>

      <nav className="flex gap-2">
        <button
          className={`rounded-md px-3 py-1 text-sm font-medium ${
            tab === "launch"
              ? "bg-accent text-accent-foreground"
              : "bg-surface text-muted-foreground hover:bg-surface-hover"
          }`}
          onClick={() => setTab("launch")}
        >
          {strings.screens.modelDetail.tabs.launch}
        </button>
        <button
          className={`rounded-md px-3 py-1 text-sm font-medium ${
            tab === "sampling"
              ? "bg-accent text-accent-foreground"
              : "bg-surface text-muted-foreground hover:bg-surface-hover"
          }`}
          onClick={() => setTab("sampling")}
        >
          {strings.screens.modelDetail.tabs.sampling}
        </button>
      </nav>

      {tab === "launch" && (
        <div className="flex flex-col gap-4">
          {/* Dirty-config banner: unsaved edits never silently win. */}
          {isDirty && (
            <div className="rounded-md border border-warn bg-warn/10 px-4 py-2 text-sm text-warn">
              {strings.screens.modelDetail.launch.dirtyBanner}
            </div>
          )}
          {justSaved && serverRunning && (
            <div className="rounded-md border border-accent bg-accent/10 px-4 py-2 text-sm text-accent">
              {strings.screens.modelDetail.launch.restartBanner}
            </div>
          )}
          {justSaved && !serverRunning && (
            <div className="rounded-md border border-pass bg-pass/10 px-4 py-2 text-sm text-pass">
              {strings.screens.modelDetail.launch.saved}
            </div>
          )}
          {saveError !== null && (
            <div className="rounded-md border border-destructive bg-destructive/10 px-4 py-2 text-sm text-destructive">
              {`${strings.screens.modelDetail.error} ${saveError}`}
            </div>
          )}

          {/* NVFP4 (or any experimental note) is shown on the Launch tab,
              not only in the model list. */}
          {experimentalNote && (
            <div className="rounded-md border border-warn bg-warn/10 px-4 py-2 text-sm text-warn">
              {strings.screens.modelDetail.launch.experimentalNote}
            </div>
          )}

          <div className="flex flex-col gap-3 rounded-lg border border-border bg-surface p-4">
            {/* Layer-budget slider with live projection. */}
            <div className="flex flex-col gap-1">
              <label className="flex items-center justify-between text-xs text-muted-foreground">
                <span>
                  {strings.screens.modelDetail.launch.gpuLayers}{" "}
                  {draftLaunch.gpu_layers !== null ? `(${draftLaunch.gpu_layers})` : ""}
                </span>
                <span>{strings.screens.modelDetail.launch.gpuLayersHint}</span>
              </label>
              <input
                type="range"
                min={0}
                max={model.metadata.block_count}
                value={draftLaunch.gpu_layers ?? 0}
                onChange={(e) =>
                  setLaunch({ ...draftLaunch, gpu_layers: Number(e.target.value) })
                }
                className="w-full accent-accent"
              />
            </div>

            <div className="grid grid-cols-2 gap-3">
              {numberField(
                strings.screens.modelDetail.launch.ctxSize,
                draftLaunch.ctx_size,
                (v) => setLaunch({ ...draftLaunch, ctx_size: v }),
              )}
              {numberField(
                strings.screens.modelDetail.launch.batchSize,
                draftLaunch.batch_size,
                (v) => setLaunch({ ...draftLaunch, batch_size: v }),
              )}
              {numberField(
                strings.screens.modelDetail.launch.ubatchSize,
                draftLaunch.ubatch_size,
                (v) => setLaunch({ ...draftLaunch, ubatch_size: v }),
              )}
              {numberField(
                strings.screens.modelDetail.launch.threads,
                draftLaunch.threads,
                (v) => setLaunch({ ...draftLaunch, threads: v }),
              )}
              {(() => {
                const cacheTypes = [
                  { value: "f16", label: strings.screens.modelDetail.launch.cacheTypeF16 },
                  { value: "bf16", label: strings.screens.modelDetail.launch.cacheTypeBF16 },
                  { value: "q8_0", label: strings.screens.modelDetail.launch.cacheTypeQ8 },
                  { value: "q4_0", label: strings.screens.modelDetail.launch.cacheTypeQ4 },
                ];
                return (
                  <label className="flex flex-col gap-1">
                    <span className="text-xs text-muted-foreground">
                      {strings.screens.modelDetail.launch.cacheTypeK}
                    </span>
                    <select
                      className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
                      value={draftLaunch.cache_type_k ?? "f16"}
                      onChange={(e) =>
                        setLaunch({
                          ...draftLaunch,
                          cache_type_k: e.target.value || null,
                        })
                      }
                    >
                      {cacheTypes.map((ct) => (
                        <option key={ct.value} value={ct.value}>
                          {ct.label}
                        </option>
                      ))}
                    </select>
                  </label>
                );
              })()}
              {(() => {
                const cacheTypes = [
                  { value: "f16", label: strings.screens.modelDetail.launch.cacheTypeF16 },
                  { value: "bf16", label: strings.screens.modelDetail.launch.cacheTypeBF16 },
                  { value: "q8_0", label: strings.screens.modelDetail.launch.cacheTypeQ8 },
                  { value: "q4_0", label: strings.screens.modelDetail.launch.cacheTypeQ4 },
                ];
                return (
                  <label className="flex flex-col gap-1">
                    <span className="text-xs text-muted-foreground">
                      {strings.screens.modelDetail.launch.cacheTypeV}
                    </span>
                    <select
                      className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
                      value={draftLaunch.cache_type_v ?? "f16"}
                      onChange={(e) =>
                        setLaunch({
                          ...draftLaunch,
                          cache_type_v: e.target.value || null,
                        })
                      }
                    >
                      {cacheTypes.map((ct) => (
                        <option key={ct.value} value={ct.value}>
                          {ct.label}
                        </option>
                      ))}
                    </select>
                  </label>
                );
              })()}
              {numberField(
                strings.screens.modelDetail.launch.nCpuMoe,
                draftLaunch.n_cpu_moe,
                (v) => setLaunch({ ...draftLaunch, n_cpu_moe: v }),
              )}
              {numberField(
                strings.screens.modelDetail.launch.mainGpu,
                draftLaunch.main_gpu,
                (v) => setLaunch({ ...draftLaunch, main_gpu: v }),
              )}
            </div>

            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={draftLaunch.no_mmap === true}
                  onChange={(e) => setLaunch({ ...draftLaunch, no_mmap: e.target.checked })}
                />
                {strings.screens.modelDetail.launch.noMmap}
              </label>
              <label className="flex items-center gap-2 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={draftLaunch.mlock === true}
                  onChange={(e) => setLaunch({ ...draftLaunch, mlock: e.target.checked })}
                />
                {strings.screens.modelDetail.launch.mlock}
              </label>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <label className="flex flex-col gap-1">
                <span className="text-xs text-muted-foreground">
                  {strings.screens.modelDetail.launch.flashAttn}
                </span>
                <select
                  className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
                  value={draftLaunch.flash_attn ?? "Auto"}
                  onChange={(e) =>
                    setLaunch({
                      ...draftLaunch,
                      flash_attn: e.target.value as LaunchParams["flash_attn"],
                    })
                  }
                >
                  <option value="On">{strings.screens.modelDetail.launch.flashAttnOn}</option>
                  <option value="Off">{strings.screens.modelDetail.launch.flashAttnOff}</option>
                  <option value="Auto">{strings.screens.modelDetail.launch.flashAttnAuto}</option>
                </select>
              </label>
              {textField(
                strings.screens.modelDetail.launch.chatTemplate,
                draftLaunch.chat_template,
                (v) => setLaunch({ ...draftLaunch, chat_template: v }),
              )}
              <p className="text-xs text-muted-foreground col-span-2">
                {strings.screens.modelDetail.launch.chatTemplateHint}
              </p>
            </div>

            {/* extra_args: unvalidated passthrough (AGENTS.md §1). */}
            <label className="flex flex-col gap-1">
              <span className="text-xs text-muted-foreground">
                {strings.screens.modelDetail.launch.extraArgs}
              </span>
              <input
                type="text"
                className="rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
                value={draftLaunch.extra_args.join(" ")}
                onChange={(e) => setLaunch({ ...draftLaunch, extra_args: e.target.value ? e.target.value.split(" ") : [] })}
              />
              <span className="text-xs text-warn">
                {strings.screens.modelDetail.launch.extraArgsWarning}
              </span>
            </label>

          {/* T-036: speculative decoding — the model's role, and the draft
              companion that changes it. The tuning controls are shown for any
              model with a draft stage: its own MTP heads, a validated
              companion, or both. */}
          <div className="flex flex-col gap-3 border-t border-border pt-4">
            <span className="text-xs font-semibold text-foreground">
              {strings.screens.modelDetail.launch.speculativeTitle}
            </span>
            <p className="text-xs text-muted-foreground">
              {model.metadata.has_mtp_heads
                ? strings.screens.modelDetail.launch.speculativeMtpNote
                : strings.screens.modelDetail.launch.speculativeNoneNote}
            </p>

            <div className="flex flex-wrap items-center gap-2">
              <span className="text-xs text-muted-foreground">
                {strings.screens.modelDetail.launch.draftLabel}
              </span>
              <span className="text-xs text-foreground break-all">
                {draftCompanionPath ?? strings.screens.modelDetail.launch.draftNone}
              </span>
              <button
                className="rounded-md border border-border bg-surface-hover px-3 py-1 text-xs font-medium text-foreground disabled:opacity-50"
                onClick={chooseDraftCompanion}
                disabled={pickingDraft}
              >
                {draftCompanionPath === null
                  ? strings.screens.modelDetail.launch.draftChoose
                  : strings.screens.modelDetail.launch.draftReplace}
              </button>
              {draftCompanionPath !== null && (
                <button
                  className="rounded-md border border-border bg-surface-hover px-3 py-1 text-xs font-medium text-foreground"
                  onClick={removeDraftCompanion}
                >
                  {strings.screens.modelDetail.launch.draftRemove}
                </button>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              {strings.screens.modelDetail.launch.draftHint}
            </p>

            {companionInfo && (
              <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-foreground">
                <span>
                  {`${strings.screens.modelDetail.launch.draftArchitectureLabel}: ${companionInfo.architecture}`}
                </span>
                <span>
                  {`${strings.screens.modelDetail.launch.speculativeExplicitType}: ${companionInfo.spec_type}`}
                </span>
                <span>
                  {`${strings.screens.modelDetail.launch.draftSizeLabel}: ${formatBytes(companionInfo.size_bytes)}`}
                </span>
              </div>
            )}

            {companionError !== null && (
              <p className="text-xs text-destructive">
                {`${strings.screens.modelDetail.launch.draftValidationFailed} ${companionError}`}
              </p>
            )}

            {(model.metadata.has_mtp_heads || companionInfo !== null) && (
              <div className="flex flex-col gap-3 border-t border-border pt-3">
                <span className="text-xs font-medium text-foreground">
                  {strings.screens.modelDetail.launch.draftTuningTitle}
                </span>
                <p className="text-xs text-muted-foreground">
                  {draftCompanionPath === null
                    ? strings.screens.modelDetail.launch.draftTuningMtpNote
                    : strings.screens.modelDetail.launch.draftTuningNote}
                </p>
                <div className="grid grid-cols-2 gap-3">
                  {numberField(
                    strings.screens.modelDetail.launch.draftNMax,
                    speculative?.n_max ?? null,
                    (v) => updateSpeculative({ n_max: v }),
                  )}
                  {numberField(
                    strings.screens.modelDetail.launch.draftNMin,
                    speculative?.n_min ?? null,
                    (v) => updateSpeculative({ n_min: v }),
                  )}
                  {numberField(
                    strings.screens.modelDetail.launch.draftPMin,
                    speculative?.p_min ?? null,
                    (v) => updateSpeculative({ p_min: v }),
                    0.01,
                  )}
                  {numberField(
                    strings.screens.modelDetail.launch.draftThreads,
                    speculative?.threads ?? null,
                    (v) => updateSpeculative({ threads: v }),
                  )}
                  {selectField(
                    strings.screens.modelDetail.launch.draftCacheTypeK,
                    speculative?.cache_type_k ?? "",
                    CACHE_TYPE_OPTIONS,
                    (value) => updateSpeculative({ cache_type_k: value === "" ? null : value }),
                  )}
                  {selectField(
                    strings.screens.modelDetail.launch.draftCacheTypeV,
                    speculative?.cache_type_v ?? "",
                    CACHE_TYPE_OPTIONS,
                    (value) => updateSpeculative({ cache_type_v: value === "" ? null : value }),
                  )}
                </div>
                <span className="text-xs text-muted-foreground">
                  {strings.screens.modelDetail.launch.draftTuningHint}
                </span>
              </div>
            )}
          </div>

          {/* Live preset preview — produced by the T-033 code path. It shows
              the section written for this model in the preset file, which is
              what the label says; the router command line is assembled at
              server start (T-040), so it cannot be shown here yet. */}
          <div className="flex flex-col gap-2 border-t border-border pt-4">
            <span className="text-xs font-semibold text-foreground">
              {strings.screens.modelDetail.launch.previewTitle}
            </span>
            <p className="text-xs text-muted-foreground">
              {strings.screens.modelDetail.launch.previewExplain}
            </p>
            {preview !== null ? (
              <pre className="max-h-64 overflow-auto rounded-md border border-border bg-input p-3 text-xs text-foreground">
                {preview}
              </pre>
            ) : previewError !== null ? (
              // The reason is the backend's, never an assumed one: the old copy
              // blamed "no active build" for every failure it could not name.
              <p className="text-xs text-warn">
                {strings.screens.modelDetail.launch.previewError} {previewError}
              </p>
            ) : (
              <Loading label={strings.screens.modelDetail.launch.previewTitle} size="sm" />
            )}
          </div>

          {/* The parameters card closes after the Save button: speculative
              decoding and the command preview are part of the same form, and
              saving them is one action (owner, 14 Sept 2026). */}
          <button
            className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-hover disabled:opacity-50"
            onClick={handleSave}
            disabled={saving}
          >
            {strings.screens.modelDetail.launch.save}
          </button>
          </div>
        </div>
      )}

      {tab === "sampling" && (
        <div className="flex flex-col gap-4">
          {/* Visible helper text, not a tooltip (T-035 acceptance). */}
          <p className="rounded-md border border-border bg-surface px-4 py-2 text-xs text-muted-foreground">
            {strings.screens.modelDetail.sampling.helper}
          </p>
          {justSaved && (
            <div className="rounded-md border border-pass bg-pass/10 px-4 py-2 text-sm text-pass">
              {strings.screens.modelDetail.sampling.saved}
            </div>
          )}
          <div className="grid grid-cols-2 gap-3 rounded-lg border border-border bg-surface p-4">
            {numberField(
              strings.screens.modelDetail.sampling.temperature,
              draftSampling.temperature,
              (v) => setDraftSampling({ ...draftSampling, temperature: v }),
              0.1,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.topP,
              draftSampling.top_p,
              (v) => setDraftSampling({ ...draftSampling, top_p: v }),
              0.01,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.topK,
              draftSampling.top_k,
              (v) => setDraftSampling({ ...draftSampling, top_k: v }),
            )}
            {numberField(
              strings.screens.modelDetail.sampling.minP,
              draftSampling.min_p,
              (v) => setDraftSampling({ ...draftSampling, min_p: v }),
              0.01,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.repeatPenalty,
              draftSampling.repeat_penalty,
              (v) => setDraftSampling({ ...draftSampling, repeat_penalty: v }),
              0.01,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.presencePenalty,
              draftSampling.presence_penalty,
              (v) => setDraftSampling({ ...draftSampling, presence_penalty: v }),
              0.01,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.frequencyPenalty,
              draftSampling.frequency_penalty,
              (v) => setDraftSampling({ ...draftSampling, frequency_penalty: v }),
              0.01,
            )}
            {numberField(
              strings.screens.modelDetail.sampling.seed,
              draftSampling.seed !== null ? Number(draftSampling.seed) : null,
              (v) => setDraftSampling({ ...draftSampling, seed: v === null ? null : BigInt(v) }),
            )}
          </div>
          <button
            className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-hover disabled:opacity-50"
            onClick={handleSave}
            disabled={saving}
          >
            {strings.screens.modelDetail.sampling.save}
          </button>
        </div>
      )}
    </section>
  );
}
