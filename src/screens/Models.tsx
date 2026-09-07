import { useEffect, useState } from "react";
import { importModels, listModels, removeModel, addWatchedFolder, listWatchedFolders, setModelPreload, setModelPinned } from "@/lib/ipc";
import { strings } from "@/lib/strings";
import type { ModelEntry, Compatibility, ModelAvailability, WatchedFolder } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Dialog } from "@/components/ui/dialog";
import { Loading } from "@/components/ui/loading";

function formatSize(bytes: number | bigint): string {
  const b = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (b >= 1024 * 1024 * 1024) {
    return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
  if (b >= 1024 * 1024) {
    return `${(b / (1024 * 1024)).toFixed(0)} MB`;
  }
  return `${(b / 1024).toFixed(0)} KB`;
}

function formatParamCount(count: bigint | null): string {
  if (count === null) return "0";
  if (count >= 1000n) {
    return `${(Number(count) / 1000).toFixed(1)}k`;
  }
  return `${Number(count)}`;
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

  const handleImportFiles = async (files: FileList) => {
    if (!files || files.length === 0) return;
    const paths: string[] = [];
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      const webkitRelativePath = (f as any).webkitRelativePath;
      paths.push(webkitRelativePath || f.name);
    }
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
    if (!folderPath.trim()) return;
    try {
      await addWatchedFolder(folderPath.trim());
      await loadModels();
      setFolderDialog(false);
      setFolderPath("");
    } catch {
      // Error handling could be added here
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
          <h1 className="text-2xl font-bold text-foreground">{strings.screens.models.title}</h1>
          <p className="text-sm text-muted-foreground">{strings.screens.models.description}</p>
        </div>
        <div className="flex gap-2">
          <label className="cursor-pointer rounded-md border border-border bg-card px-4 py-2 text-sm font-medium text-foreground hover:bg-muted">
            {strings.screens.models.addModel}
            <input
              type="file"
              accept=".gguf"
              multiple
              className="hidden"
              onChange={(e) => {
                if (e.target.files) {
                  handleImportFiles(e.target.files);
                }
              }}
            />
          </label>
          <button
            className="rounded-md border border-border bg-card px-4 py-2 text-sm font-medium text-foreground hover:bg-muted"
            onClick={() => setFolderDialog(true)}
          >
            {strings.screens.models.addFolder}
          </button>
        </div>
      </header>

      {presetChanged && (
        <div className="flex items-center gap-2 rounded-md border border-amber-900 bg-amber-950/30 px-4 py-2 text-sm text-amber-300">
          <span>{strings.screens.models.presetChanged}</span>
          <button
            className="ml-2 underline hover:text-amber-200"
            onClick={() => setPresetChanged(false)}
          >
            {strings.screens.models.dismiss}
          </button>
        </div>
      )}

      {importing && (
        <div className="flex items-center gap-2 rounded-md border border-sky-900 bg-sky-950/30 px-4 py-2 text-sm text-sky-300">
          <Loading label={strings.screens.models.importing} size="sm" />
        </div>
      )}

      <div className="flex-1 overflow-y-auto rounded-lg border border-border bg-card">
        {models.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-4 p-12 text-center">
            <div className="text-6xl text-muted-foreground">{strings.screens.models.emptyIcon}</div>
            <div>
              <h2 className="text-lg font-semibold text-foreground">{strings.screens.models.emptyTitle}</h2>
              <p className="mt-1 text-sm text-muted-foreground">{strings.screens.models.emptyBody}</p>
            </div>
            <label className="cursor-pointer rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-700">
              {strings.screens.models.addFirstModel}
              <input
                type="file"
                accept=".gguf"
                multiple
                className="hidden"
                onChange={(e) => {
                if (e.target.files) {
                  handleImportFiles(e.target.files);
                }
              }}
              />
            </label>
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
                    <div className="mt-1 text-xs text-red-400">{model.file_path}</div>
                  )}
                </div>
                <div className="flex items-center gap-4">
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
                    className="rounded-md border border-border bg-card px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted"
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
        <div className="rounded-lg border border-border bg-card p-4">
          <h2 className="text-sm font-semibold text-foreground">{strings.screens.models.watchedFolders}</h2>
          <div className="mt-2 space-y-1">
            {watchedFolders.map((folder) => (
              <div key={folder.path} className="flex items-center justify-between text-sm text-muted-foreground">
                <span>{folder.path}</span>
                <span className="text-xs">{folder.model_count} {strings.screens.models.modelsCount}</span>
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
            <div className="rounded-md border border-amber-900 bg-amber-950/30 p-3 text-xs text-amber-300">
              {strings.screens.models.removeRetention}
            </div>
            <div className="flex justify-end gap-2">
              <button
                className="rounded-md border border-border bg-card px-4 py-2 text-sm text-foreground hover:bg-muted"
                onClick={() => setRemoveDialog(null)}
              >
                {strings.screens.models.cancel}
              </button>
              <button
                className="rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700"
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
            className="w-full rounded-md border border-border bg-input px-3 py-2 text-sm text-foreground"
            placeholder={strings.screens.models.folderPathPlaceholder}
            value={folderPath}
            onChange={(e) => setFolderPath(e.target.value)}
          />
          <div className="flex justify-end gap-2">
            <button
              className="rounded-md border border-border bg-card px-4 py-2 text-sm text-foreground hover:bg-muted"
              onClick={() => setFolderDialog(false)}
            >
              {strings.screens.models.cancel}
            </button>
            <button
              className="rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-700"
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