import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Loading } from "@/components/ui/loading";
import { Separator } from "@/components/ui/separator";
import { probeEnvironment } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { strings } from "@/lib/strings";
import type { CheckStatus, EnvironmentReport, HealthCheck } from "@/lib/types";

// Health-check screen (docs/TASKS.md T-011). This is the first-run view
// of the Runtime route, not an eighth route — `src/lib/routes.ts` still
// defines seven (T-005) and nothing here adds to that list. Replaces the
// scaffolded placeholder T-005 left behind (`RuntimeScreen` used to
// render nothing but `strings.emptyScreenNote`).
//
// Scope discipline (docs/WORKFLOW.md §3): traffic lights, remediation
// text and a manual refresh button only. No starting/stopping the
// server (that is `ServerState`/the orchestrator, a later task) and no
// polling interval — "manual refresh" means a button, not a timer.

const copy = strings.screens.runtime;
const healthCopy = copy.healthCheck;

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

export function RuntimeScreen() {
  const [report, setReport] = useState<EnvironmentReport | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [hasError, setHasError] = useState(false);

  // Fetches and updates state only from the promise's own callbacks —
  // no setState call is synchronous within the function body itself, so
  // it is safe to invoke directly from the mount effect below
  // (react-hooks/set-state-in-effect). The synchronous "start loading"
  // setState calls a refresh needs live in `handleRefresh`, not here.
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

  // Manual refresh only (docs/WORKFLOW.md §3) — fetched once on mount to
  // populate the first-run view, and again only when this handler runs
  // from the button below. No `setInterval`, no polling.
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
