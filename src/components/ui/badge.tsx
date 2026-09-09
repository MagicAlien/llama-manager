import type { ReactNode } from "react";

interface BadgeProps {
  children: ReactNode;
  variant?: "default" | "success" | "warning" | "error" | "info";
  className?: string;
}

export function Badge({ children, variant = "default", className = "" }: BadgeProps) {
  // Token colours only (T-005's contract): Tailwind's default palette
  // does not exist in this app — tailwind.config.js replaces it.
  const variants = {
    default: "bg-surface-hover text-muted-foreground",
    success: "bg-pass/10 text-pass",
    warning: "bg-warn/10 text-warn",
    error: "bg-destructive/10 text-destructive",
    info: "bg-accent/10 text-accent",
  };

  return (
    <span className={`inline-flex items-center rounded-full px-2 py-1 text-xs font-medium ${variants[variant]} ${className}`}>
      {children}
    </span>
  );
}