import { useEffect, useState } from "react";
import { importModels, listModels, removeModel, addWatchedFolder, removeWatchedFolder, listWatchedFolders, setModelPreload, setModelPinned, rescanModels } from "@/lib/ipc";
import { strings } from "@/lib/strings";
import type { ModelEntry, Compatibility, ModelAvailability, WatchedFolder } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Dialog } from "@/components/ui/dialog";
import { Loading } from "@/components/ui/loading";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

function formatSize(bytes: bigint | number): string {
  const b = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (b >= 1024 * 1024 * 1024) {
    return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
  if (b >= 1024 * 1024) {
    return `${(b / (1024 * 1024)).toFixed(0)} MB`;
  }
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
  if (c >= 1_000) {
    const k = c / 1_000;
    return `${Number.isInteger(k) ? k : k.toFixed(1)}k`;
  }
  return `${c}`;
}

function compatibilityBadge(compat: Compatibility) {
  if (compat === "Supported") {
    return <Badge variant="success">{strings.screens.models.badges.supported}</Badge>;
  }
  if ("SupportedWithWarnings" in compat) {
    const hasExperimental = compat.SupportedWithWarnings.some((n) => n.experimental);
    if (hasExperimental) {
      return <Badge variant="info">{strings.screens.models.badges.experimental}</Badge>;
    }
    return <Badge variant="warning">{strings.screens.models.badges.warnings}</Badge>;
  }
  return <Badge variant="error">{strings.screens.models.badges.unsupported}</Badge>;
}

function availabilityBadge(availability: ModelAvailability) {
  switch (availability) {
    case "Present":
      return <Badge variant="success">{strings.screens.models.badges.present}</Badge>;
    case "Missing":
      return <Badge variant="error">{strings.screens.models.badges.missing}</Badge>;
    case "Unreadable":
      return <Badge variant="warning">{strings.screens.models.badges.unreadable}</Badge>;
  }
}

export function ModelsScreen() {
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [watchedFolders, setWatchedFolders] = useState<WatchedFolder[]>([]);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [removeDialog, setRemoveDialog] = useState<ModelEntry | null>(null);
  const [folderDialog, setFolderDialog] = useState(false);
  const [folderPath, setFolderPath] = useState("");
  const [presetChanged, setPresetChanged] = useState(false);
  const [rescanning, setRescanning] = useState(false);

  const loadModels = async () => {
    try {
      const [models, folders] = await Promise.all([listModels(), listWatchedFolders()]);
      setModels(models);
      setWatchedFolders(folders);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadModels();
  }, []);

  const handleImportFiles = async (paths: string[]) => {
    if (paths.length === 0) return;

    setImporting(true);
    try {
      await importModels(paths);
      await loadModels();
      setPresetChanged(true);
    } finally {
      setImporting(false);
    }
  };

  const pickAndImport = async () => {
    const selected = await openDialog({
      multiple: true,
      filters: [{ name: strings.screens.models.ggufFilterName, extensions: ["gguf"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    await handleImportFiles(paths);
  };

  const handleRemove = async (model: ModelEntry) => {
    try {
      await removeModel(model.id);
      await loadModels();
      setPresetChanged(true);
    } finally {
      setRemoveDialog(null);
    }
  };

  const handleAddFolder = async () => {
    // Prefer the native folder picker; the typed path is the fallback.
    const picked = folderPath.trim()
      ? folderPath.trim()
      : ((await openDialog({ directory: true })) ?? "");
    if (!picked) return;
    try {
      await addWatchedFolder(picked);
      await loadModels();
      setFolderDialog(false);
      setFolderPath("");
      setPresetChanged(true);
    } catch {
      // The IPC error surfaces via the rejected promise; the folder dialog
      // stays open so the user can retry with a different path.
    }
  };

  const pickFolder = async () => {
    const picked = await openDialog({ directory: true });
    if (picked) setFolderPath(picked);
  };

  const handleRescan = async () => {
    setRescanning(true);
    try {
      await rescanModels();
      await loadModels();
    } finally {
      setRescanning(false);
    }
  };

  const handleRemoveFolder = async (path: string) => {
    try {
      await removeWatchedFolder(path);
      await loadModels();
      setPresetChanged(true);
    } catch {
      // Rejected promise keeps the folder in the list; the user sees no
      // change rather than a row that disappeared server-side but not
      // client-side.
    }
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loading label={strings.screens.models.loadingModels} size="md" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold text-foreground">{strings.screens.models.title}</h1>
          <p className="text-sm text-muted-foreground">{strings.screens.models.description}</p>
        </div>
        <div className="flex gap-2">
          <button
            className="cursor-pointer rounded-md border border-border bg-surface px-4 py-2 text-sm font-medium text-foreground hover:bg-surface-hover"
            onClick={pickAndImport}
          >
            {strings.screens.models.addModel}
          </button>
          <button
            className="rounded-md border border-border bg-surface px-4 py-2 text-sm font-medium text-foreground hover:bg-surface-hover"
            onClick={() => setFolderDialog(true)}
          >
            {strings.screens.models.addFolder}
          </button>
          <button
            className="cursor-pointer rounded-md border border-border bg-surface px-4 py-2 text-sm font-medium text-foreground hover:bg-surface-hover"
            onClick={handleRescan}
            disabled={rescanning}
          >
            {rescanning ? strings.screens.models.rescanning : strings.screens.models.rescan}
          </button>
        </div>
      </header>

      {presetChanged && (
        <div className="flex items-center gap-2 rounded-md border border-warn bg-warn/10 px-4 py-2 text-sm text-warn">
          <span>{strings.screens.models.presetChanged}</span>
          <button
            className="ml-2 underline hover:text-foreground"
            onClick={() => setPresetChanged(false)}
          >
            {strings.screens.models.dismiss}
          </button>
        </div>
      )}

      {importing && (
        <div className="flex items-center gap-2 rounded-md border border-accent bg-accent/10 px-4 py-2 text-sm text-accent">
          <Loading label={strings.screens.models.importing} size="sm" />
        </div>
      )}

      <div className="flex-1 overflow-y-auto rounded-lg border border-border bg-surface">
        {models.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-4 p-12 text-center">
            <div className="text-xl text-muted-foreground">{strings.screens.models.emptyIcon}</div>
            <div>
              <h2 className="text-lg font-semibold text-foreground">{strings.screens.models.emptyTitle}</h2>
              <p className="mt-1 text-sm text-muted-foreground">{strings.screens.models.emptyBody}</p>
            </div>
            <button
              className="cursor-pointer rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-hover"
              onClick={pickAndImport}
            >
              {strings.screens.models.addFirstModel}
            </button>
          </div>
        ) : (
          <div className="divide-y divide-border">
            {models.map((model) => (
              <div key={model.id} className="flex items-center gap-4 p-4">
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-foreground">{model.display_name}</span>
                    {compatibilityBadge(model.compatibility)}
                    {availabilityBadge(model.availability)}
                    {model.duplicate_of && (
                      <Badge variant="warning">{strings.screens.models.duplicate}</Badge>
                    )}
                    {model.metadata.is_moe && (
                      <Badge variant="info">{strings.screens.models.moe}</Badge>
                    )}
                  </div>
                  <div className="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
                    <span>{formatSize(model.size_bytes)}</span>
                    <span>{formatParamCount(model.metadata.param_count)} {strings.screens.models.params}</span>
                    <span>{model.metadata.quantization}</span>
                    <span>{model.metadata.architecture}</span>
                    {model.shard_paths.length > 0 && (
                      <span>{model.shard_paths.length + 1} {strings.screens.models.shards}</span>
                    )}
                  </div>
                  {model.availability === "Missing" && (
                    <div className="mt-1 text-xs text-destructive">{model.file_path}</div>
                  )}
                </div>
                <div className="flex items-center gap-4">
                  <a
                    href={`#/models/detail/${model.id}`}
                    className="rounded-md border border-border bg-surface px-3 py-1 text-xs font-medium text-foreground hover:bg-surface-hover"
                  >
                    {strings.screens.models.details}
                  </a>
                  <label className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Switch
                      checked={model.preload}
                      onChange={(checked) => {
                        if (model.availability !== "Present") return;
                        setModelPreload(model.id, checked).then(loadModels);
                      }}
                      disabled={model.availability !== "Present"}
                    />
                    {strings.screens.models.preload}
                  </label>
                  <label className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Switch
                      checked={model.pinned}
                      onChange={(checked) => {
                        if (model.availability !== "Present") return;
                        setModelPinned(model.id, checked).then(loadModels);
                      }}
                      disabled={model.availability !== "Present"}
                    />
                    {strings.screens.models.pinned}
                  </label>
                  <button
                    className="rounded-md border border-border bg-surface px-3 py-1 text-xs font-medium text-foreground hover:bg-surface-hover"
                    onClick={() => setRemoveDialog(model)}
                  >
                    {strings.screens.models.remove}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {watchedFolders.length > 0 && (
        <div className="rounded-lg border border-border bg-surface p-4">
          <h2 className="text-sm font-semibold text-foreground">{strings.screens.models.watchedFolders}</h2>
          <div className="mt-2 space-y-1">
            {watchedFolders.map((folder) => (
              <div key={folder.path} className="flex items-center justify-between text-sm text-muted-foreground">
                <span>{folder.path}</span>
                <div className="flex items-center gap-3">
                  <span className="text-xs">{folder.model_count} {strings.screens.models.modelsCount}</span>
                  <button
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => handleRemoveFolder(folder.path)}
                  >
                    {strings.screens.models.removeFolder}
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      <Dialog
        open={!!removeDialog}
        onClose={() => setRemoveDialog(null)}
        title={strings.screens.models.removeTitle}
      >
        {removeDialog && (
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              {strings.screens.models.removeBody}
            </p>
            <p className="text-sm font-medium text-foreground">{removeDialog.display_name}</p>
            <div className="rounded-md border border-warn bg-warn/10 p-3 text-xs text-warn">
              {strings.screens.models.removeRetention}
            </div>
            <div className="flex justify-end gap-2">
              <button
                className="rounded-md border border-border bg-surface px-4 py-2 text-sm text-foreground hover:bg-surface-hover"
                onClick={() => setRemoveDialog(null)}
              >
                {strings.screens.models.cancel}
              </button>
              <button
                className="rounded-md bg-destructive px-4 py-2 text-sm font-medium text-destructive-foreground hover:bg-destructive/80"
                onClick={() => handleRemove(removeDialog)}
              >
                {strings.screens.models.confirmRemove}
              </button>
            </div>
          </div>
        )}
      </Dialog>

      <Dialog
        open={folderDialog}
        onClose={() => setFolderDialog(false)}
        title={strings.screens.models.addFolderTitle}
      >
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            {strings.screens.models.addFolderBody}
          </p>
          <input
            type="text"
            className="w-full rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground"
            placeholder={strings.screens.models.folderPathPlaceholder}
            value={folderPath}
            onChange={(e) => setFolderPath(e.target.value)}
          />
          <div className="flex justify-end gap-2">
            <button
              className="mr-auto rounded-md border border-border bg-surface px-4 py-2 text-sm text-foreground hover:bg-surface-hover"
              onClick={pickFolder}
            >
              {strings.screens.models.browseFolder}
            </button>
            <button
              className="rounded-md border border-border bg-surface px-4 py-2 text-sm text-foreground hover:bg-surface-hover"
              onClick={() => setFolderDialog(false)}
            >
              {strings.screens.models.cancel}
            </button>
            <button
              className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-hover"
              onClick={handleAddFolder}
            >
              {strings.screens.models.addFolderBtn}
            </button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}