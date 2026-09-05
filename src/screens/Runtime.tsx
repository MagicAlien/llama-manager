import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Loading } from "@/components/ui/loading";
import { Separator } from "@/components/ui/separator";
import { VersionManagementView } from "@/components/VersionManagement";
import { listRuntimes, probeEnvironment } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { strings } from "@/lib/strings";
import type { CheckStatus, EnvironmentReport, HealthCheck } from "@/lib/types";

// Runtime route (docs/TASKS.md T-011, T-024).
//
// Two views:
// - Health-check screen: first-run view when no builds are installed.
// - Version management screen: main view after at least one build is installed.
//
// Both live in this file because they share the route; the health-check
// is not an eighth route (docs/TASKS.md T-011).

const copy = strings.screens.runtime;
const healthCopy = copy.healthCheck;
const versionsCopy = copy.versions;

const STATUS_DOT_CLASS: Record<CheckStatus, string> = {
  Pass: "bg-pass",
  Warn: "bg-warn",
  Fail: "bg-destructive",
};

function checkLabel(id: string): string {
  const known = healthCopy.checkLabels as Record<string, string>;
  return known[id] ?? id.replace(/_/g, " ");
}

function statusLabel(status: CheckStatus): string {
  return healthCopy.statusLabels[status];
}

function CheckRow({ check }: { check: HealthCheck }) {
  return (
    <li
      data-status={check.status}
      className="flex flex-col gap-1 rounded-md border border-border bg-surface p-3"
    >
      <div className="flex items-center gap-2">
        <span
          aria-hidden="true"
          data-status={check.status}
          className={cn("h-2 w-2 shrink-0 rounded-full", STATUS_DOT_CLASS[check.status])}
        />
        <span className="text-sm font-medium text-foreground">{checkLabel(check.id)}</span>
        <span className="text-xs text-muted-foreground">{statusLabel(check.status)}</span>
      </div>
      <p className="text-xs text-muted-foreground">{check.message}</p>
      {check.remediation ? (
        <p className="text-xs text-accent">
          {healthCopy.remediationLabel} {check.remediation}
        </p>
      ) : null}
    </li>
  );
}

function HealthCheckView() {
  const [report, setReport] = useState<EnvironmentReport | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [hasError, setHasError] = useState(false);

  const fetchEnvironment = useCallback(() => {
    probeEnvironment()
      .then((result) => {
        setReport(result);
        setHasError(false);
      })
      .catch(() => {
        setHasError(true);
      })
      .finally(() => {
        setIsLoading(false);
      });
  }, []);

  useEffect(() => {
    fetchEnvironment();
  }, [fetchEnvironment]);

  const handleRefresh = useCallback(() => {
    setIsLoading(true);
    setHasError(false);
    fetchEnvironment();
  }, [fetchEnvironment]);

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
          <p className="text-sm text-muted-foreground">{copy.description}</p>
        </div>
        <Button onClick={handleRefresh} disabled={isLoading}>
          {healthCopy.refresh}
        </Button>
      </div>

      <Separator />

      <h2 className="text-lg font-semibold text-foreground">{healthCopy.heading}</h2>

      {report ? (
        <ul className="flex flex-col gap-2">
          {report.checks.map((check) => (
            <CheckRow key={check.id} check={check} />
          ))}
        </ul>
      ) : isLoading ? (
        <Loading label={healthCopy.loadingLabel} />
      ) : hasError ? (
        <p className="text-sm text-destructive">{healthCopy.errorBody}</p>
      ) : null}
    </section>
  );
}

export function RuntimeScreen() {
  const [hasBuilds, setHasBuilds] = useState<boolean | null>(null);

  useEffect(() => {
    listRuntimes()
      .then((builds) => setHasBuilds(builds.length > 0))
      .catch(() => setHasBuilds(false));
  }, []);

  if (hasBuilds === null) {
    return (
      <section className="flex flex-col gap-4">
        <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
        <Loading label={versionsCopy.loadingLabel ?? "Loading runtime info…"} />
      </section>
    );
  }

  if (hasBuilds) {
    return <VersionManagementView />;
  }

  return <HealthCheckView />;
}
