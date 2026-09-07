import type { ReactNode } from "react";

interface BadgeProps {
  children: ReactNode;
  variant?: "default" | "success" | "warning" | "error" | "info";
  className?: string;
}

export function Badge({ children, variant = "default", className = "" }: BadgeProps) {
  const variants = {
    default: "bg-muted text-muted-foreground",
    success: "bg-emerald-900/40 text-emerald-300",
    warning: "bg-amber-900/40 text-amber-300",
    error: "bg-red-900/40 text-red-300",
    info: "bg-sky-900/40 text-sky-300",
  };

  return (
    <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${variants[variant]} ${className}`}>
      {children}
    </span>
  );
}