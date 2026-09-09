import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Loading } from "@/components/ui/loading";
import { Separator } from "@/components/ui/separator";
import { VersionManagementView } from "@/components/VersionManagement";
import { probeEnvironment } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { strings } from "@/lib/strings";
import type { CheckStatus, EnvironmentReport, HealthCheck } from "@/lib/types";

// Runtime route (docs/TASKS.md T-011, T-024).
//
// Layout (9 Sept 2026, owner decision — supersedes T-011/T-024's
// mutually-exclusive "first-run vs. version-management" split):
// - A single header (title + description + one Refresh) for the whole route.
// - The version-management section (T-024) is ALWAYS shown first — its
//   empty state ("No runtime builds installed yet") is the first-run call
//   to action, and its grid appears once a build exists.
// - The environment health section (T-011) is ALWAYS shown below it, not
//   only on first run — the GPU/driver/CUDA/disk/port checks stay useful
//   after a build is installed.
//
// The Refresh button re-runs the environment probe AND re-fetches the
// version-management data (via the `nonce` prop), so one control refreshes
// the whole screen.

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

// Presentational: the parent owns the probe state and the Refresh action,
// so a single header control refreshes both this section and the
// version-management section below it.
function HealthCheckView({
  report,
  isLoading,
  hasError,
}: {
  report: EnvironmentReport | null;
  isLoading: boolean;
  hasError: boolean;
}) {
  return (
    <section className="flex flex-col gap-4">
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
  const [report, setReport] = useState<EnvironmentReport | null>(null);
  const [probeLoading, setProbeLoading] = useState(true);
  const [probeError, setProbeError] = useState(false);
  // Bumped on every Refresh to re-fetch the version-management data.
  const [vmNonce, setVmNonce] = useState(0);

  const fetchEnvironment = useCallback(() => {
    setProbeLoading(true);
    setProbeError(false);
    probeEnvironment()
      .then((result) => {
        setReport(result);
      })
      .catch(() => {
        setProbeError(true);
      })
      .finally(() => {
        setProbeLoading(false);
      });
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    fetchEnvironment();
  }, [fetchEnvironment]);

  const handleRefresh = useCallback(() => {
    fetchEnvironment();
    setVmNonce((n) => n + 1);
  }, [fetchEnvironment]);

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
          <p className="text-sm text-muted-foreground">{copy.description}</p>
        </div>
        <Button onClick={handleRefresh} disabled={probeLoading}>
          {healthCopy.refresh}
        </Button>
      </div>

      <Separator />

      <VersionManagementView nonce={vmNonce} />

      <HealthCheckView report={report} isLoading={probeLoading} hasError={probeError} />
    </section>
  );
}
