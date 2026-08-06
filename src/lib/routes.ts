// Single source for the seven navigable routes (docs/TASKS.md T-005).
// Display labels live in src/lib/strings.ts (AGENTS.md invariant 7) —
// this file only maps an id to its URL hash. No screen here does IPC,
// state or data fetching (docs/WORKFLOW.md §3 scope discipline); that
// is later tasks' work.
export const ROUTE_IDS = [
  "dashboard",
  "runtime",
  "models",
  "modelDetail",
  "api",
  "logs",
  "settings",
] as const;

export type RouteId = (typeof ROUTE_IDS)[number];

interface RouteDefinition {
  id: RouteId;
  hash: string;
}

export const ROUTES: readonly RouteDefinition[] = [
  { id: "dashboard", hash: "#/dashboard" },
  { id: "runtime", hash: "#/runtime" },
  { id: "models", hash: "#/models" },
  { id: "modelDetail", hash: "#/models/detail" },
  { id: "api", hash: "#/api" },
  { id: "logs", hash: "#/logs" },
  { id: "settings", hash: "#/settings" },
];

export const DEFAULT_ROUTE_ID: RouteId = "dashboard";
