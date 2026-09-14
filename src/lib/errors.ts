// T-036 — readable text for a rejected IPC call.
//
// Every command in `docs/CONTRACTS.md` §4 rejects with the serialized
// `AppError`, whose shape is `{ kind: "...", detail: { ... } }` (internally
// tagged, `core/types.rs`) — never a string and never an `Error` instance.
// Rendering the rejection directly is how the UI showed "[object Object]" in
// place of the reason, which is worse than showing nothing: the user cannot
// act on it and the developer cannot diagnose it.
//
// `message` is preferred when the variant carries it, because the Rust side
// writes those strings for humans (each variant's `#[error("...")]`,
// `AGENTS.md` invariant 6's `AppError::message`). The other detail fields are
// rendered as `key: value` pairs so a variant added later degrades into
// something readable instead of a blank.

import type { AppError } from "./types";

export function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error === null || error === undefined) return "";
  if (typeof error !== "object") return String(error);

  const candidate = error as Partial<AppError> & {
    kind?: unknown;
    detail?: Record<string, unknown>;
  };

  if (typeof candidate.kind !== "string") {
    // Not an AppError: probably a Tauri-level failure ("command not found").
    // JSON is ugly but truthful, and only appears where the shape is unknown.
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }

  const detail = candidate.detail ?? {};
  if (typeof detail.message === "string" && detail.message !== "") {
    return detail.message;
  }

  const parts = Object.entries(detail)
    .filter(([, value]) => value !== null && value !== undefined)
    .map(([key, value]) =>
      typeof value === "object" ? `${key}: ${JSON.stringify(value)}` : `${key}: ${String(value)}`,
    );

  return parts.length > 0 ? `${candidate.kind} — ${parts.join(", ")}` : candidate.kind;
}
