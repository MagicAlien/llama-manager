import { cn } from "@/lib/cn";

export interface LoadingProps {
  /**
   * What is loading. Required, deliberately: a spinner with no label
   * has no accessible name and tells the user nothing (docs/TASKS.md
   * T-005 acceptance: "the loading component's label prop is
   * required — a bare spinner is a TypeScript error"). See
   * `loading.test.tsx` for the type-level assertion.
   */
  label: string;
  size?: "sm" | "md";
  className?: string;
}

export function Loading({ label, size = "md", className }: LoadingProps) {
  return (
    <div role="status" className={cn("inline-flex items-center gap-2", className)}>
      <span
        aria-hidden="true"
        className={cn(
          "animate-spin rounded-full border-2 border-border border-t-accent",
          size === "sm" ? "h-4 w-4" : "h-6 w-6",
        )}
      />
      <span className="text-sm text-muted-foreground">{label}</span>
    </div>
  );
}
