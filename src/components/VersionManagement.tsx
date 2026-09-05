import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Loading } from "@/components/ui/loading";
import { Separator } from "@/components/ui/separator";
import {
  activateRuntime,
  checkForUpdates,
  getServerState,
  installRuntime,
  listRuntimes,
  removeRuntime,
} from "@/lib/ipc";
import { strings } from "@/lib/strings";
import type {
  AvailableRelease,
  Backend,
  RuntimeBuild,
  ServerState,
} from "@/lib/types";

const copy = strings.screens.runtime;
const versionsCopy = copy.versions;

function backendLabel(backend: Backend): string {
  switch (backend.kind) {
    case "Cuda":
      return `CUDA ${backend.major}`;
    case "Vulkan":
      return "Vulkan";
    case "Cpu":
      return "CPU";
  }
}

function formatInstalledAt(iso: string): string {
  try {
    const date = new Date(iso);
    return date.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

function isServerStopped(state: ServerState): boolean {
  return state === "Stopped";
}

function BuildCard({
  build,
  isServerRunning,
  isLastBuild,
  latestForBackend,
  onActivate,
  onRemove,
  onInstall,
}: {
  build: RuntimeBuild;
  isServerRunning: boolean;
  isLastBuild: boolean;
  latestForBackend: AvailableRelease | null;
  onActivate: (build: RuntimeBuild) => void;
  onRemove: (build: RuntimeBuild) => void;
  onInstall: (tag: string, backend: Backend) => void;
}) {
  const [activating, setActivating] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [installing, setInstalling] = useState(false);

  const canActivate = !isServerRunning && !build.is_active && !activating;
  const canRemove = !isServerRunning && !isLastBuild && !removing;
  const canInstall = !installing;

  const handleActivate = async () => {
    setActivating(true);
    try {
      await onActivate(build);
    } finally {
      setActivating(false);
    }
  };

  const handleRemove = async () => {
    setRemoving(true);
    try {
      await onRemove(build);
    } finally {
      setRemoving(false);
    }
  };

  const handleInstall = async () => {
    if (latestForBackend) {
      setInstalling(true);
      try {
        await onInstall(latestForBackend.build_tag, build.backend);
      } finally {
        setInstalling(false);
      }
    }
  };

  const isUndetermined = build.registration_channel === "Undetermined";

  return (
    <article
      data-active={build.is_active}
      className="flex flex-col gap-3 rounded-lg border border-border bg-surface p-4"
    >
      <div className="flex items-start justify-between gap-2">
        <div>
          <h3 className="text-base font-semibold text-foreground">
            {build.build_tag}
          </h3>
          <p className="text-xs text-muted-foreground">
            {versionsCopy.backendLabel}{versionsCopy.colon} {backendLabel(build.backend)}
          </p>
        </div>
        {build.is_active && (
          <span className="shrink-0 rounded-full bg-accent px-2 py-0.5 text-xs font-medium text-accent-foreground">
            {versionsCopy.activeBadge}
          </span>
        )}
      </div>

      <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
        <dt className="text-muted-foreground">{versionsCopy.installPathLabel}</dt>
        <dd className="text-foreground">{build.install_path}</dd>
        <dt className="text-muted-foreground">{versionsCopy.installedAtLabel}</dt>
        <dd className="text-foreground">{formatInstalledAt(build.installed_at)}</dd>
      </dl>

      {isUndetermined && (
        <p className="rounded-md bg-amber-100 p-2 text-xs text-amber-800 dark:bg-amber-900 dark:text-amber-200">
          ⚠ {versionsCopy.undeterminedFlag}
        </p>
      )}

      {latestForBackend && (
        <div className="flex items-center justify-between text-xs">
          <span className="text-muted-foreground">
            {versionsCopy.currentVersion}{versionsCopy.colon} {build.build_tag} {versionsCopy.arrow} {versionsCopy.latestVersion}{versionsCopy.colon} {latestForBackend.build_tag}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={handleInstall}
            disabled={!canInstall}
          >
            {installing
              ? versionsCopy.installingLabel
              : versionsCopy.installButton}
          </Button>
        </div>
      )}

      {build.verified_flags.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {build.verified_flags.length} {versionsCopy.flagsVerified}
        </p>
      )}

      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={handleActivate}
          disabled={!canActivate}
        >
          {activating
            ? versionsCopy.activatingLabel
            : versionsCopy.activateButton}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={handleRemove}
          disabled={!canRemove}
        >
          {removing
            ? versionsCopy.removingLabel
            : versionsCopy.removeButton}
        </Button>
      </div>
    </article>
  );
}

export function VersionManagementView() {
  const [builds, setBuilds] = useState<RuntimeBuild[]>([]);
  const [serverState, setServerState] = useState<ServerState>("Stopped");
  const [latestReleases, setLatestReleases] = useState<AvailableRelease[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  const isServerRunning = !isServerStopped(serverState);

  const fetchAll = useCallback(async () => {
    setIsLoading(true);
    try {
      const [runtimes, state, releases] = await Promise.all([
        listRuntimes(),
        getServerState(),
        checkForUpdates().catch(() => [] as AvailableRelease[]),
      ]);
      setBuilds(runtimes);
      setServerState(state);
      setLatestReleases(releases);
    } catch {
      setMessage({ type: "error", text: versionsCopy.error });
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    fetchAll();
  }, [fetchAll]);

  const handleActivate = async (build: RuntimeBuild) => {
    try {
      await activateRuntime(build.build_tag, build.backend);
      setMessage({ type: "success", text: versionsCopy.activateSuccess });
      await fetchAll();
    } catch {
      setMessage({ type: "error", text: versionsCopy.error });
    }
  };

  const handleRemove = async (build: RuntimeBuild) => {
    try {
      await removeRuntime(build.build_tag, build.backend);
      setMessage({ type: "success", text: versionsCopy.removeSuccess });
      await fetchAll();
    } catch {
      setMessage({ type: "error", text: versionsCopy.error });
    }
  };

  const handleInstall = async (tag: string, backend: Backend) => {
    try {
      await installRuntime(tag, backend);
      setMessage({ type: "success", text: versionsCopy.installSuccess });
      await fetchAll();
    } catch {
      setMessage({ type: "error", text: versionsCopy.error });
    }
  };

  const getLatestForBackend = (backend: Backend): AvailableRelease | null => {
    return latestReleases.find((r) => {
      if (r.backend.kind === backend.kind) {
        if (backend.kind === "Cuda" && r.backend.kind === "Cuda") {
          return r.backend.major === backend.major;
        }
        return true;
      }
      return false;
    }) ?? null;
  };

  if (isLoading) {
    return (
      <section className="flex flex-col gap-4">
        <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
        <Loading label={versionsCopy.loadingLabel ?? "Loading builds…"} />
      </section>
    );
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
          <p className="text-sm text-muted-foreground">{copy.description}</p>
        </div>
        <Button onClick={fetchAll}>{versionsCopy.refresh}</Button>
      </div>

      <Separator />

      <h2 className="text-lg font-semibold text-foreground">
        {versionsCopy.heading}
      </h2>

      {isServerRunning && (
        <p className="rounded-md bg-amber-100 p-2 text-xs text-amber-800 dark:bg-amber-900 dark:text-amber-200">
          {versionsCopy.notStoppedReason}
        </p>
      )}

      {message && (
        <p
          className={`rounded-md p-2 text-xs ${
            message.type === "success"
              ? "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200"
              : "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200"
          }`}
        >
          {message.text}
        </p>
      )}

      {builds.length === 0 ? (
        <p className="text-sm text-muted-foreground">{versionsCopy.empty}</p>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {builds.map((build) => (
            <BuildCard
              key={`${build.build_tag}-${backendLabel(build.backend)}`}
              build={build}
              isServerRunning={isServerRunning}
              isLastBuild={builds.length === 1}
              latestForBackend={getLatestForBackend(build.backend)}
              onActivate={handleActivate}
              onRemove={handleRemove}
              onInstall={handleInstall}
            />
          ))}
        </div>
      )}
    </section>
  );
}
