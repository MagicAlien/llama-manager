import clsx, { type ClassValue } from "clsx";

// Thin wrapper kept as the single class-name-composition call site
// across components (T-005). Not a tailwind-merge-based `cn` — nothing
// in this codebase yet stacks conflicting utility classes that need
// merge-precedence resolution, and `tailwind-merge` is not on
// `PLAN.md` §3's approved list, so it is not pulled in speculatively.
export function cn(...inputs: ClassValue[]): string {
  return clsx(inputs);
}
