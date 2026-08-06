import type { ButtonHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";

// shadcn/ui-style primitive, reimplemented against this project's own
// tokens (T-005) rather than copied from a template. No
// `@radix-ui/react-slot` / `asChild` polymorphism: every use in this
// task is a real `<button>`, so that dependency isn't pulled in
// speculatively (see PR Dependencies — only the two Radix packages
// this task actually uses, `react-separator` and `react-tooltip`, are
// added).
const buttonVariants = cva(
  "inline-flex items-center justify-center rounded-md font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-accent text-accent-foreground hover:bg-accent-hover",
        ghost: "bg-transparent text-foreground hover:bg-surface-hover",
        outline: "border border-border bg-transparent text-foreground hover:bg-surface-hover",
      },
      size: {
        default: "h-9 px-4 text-sm",
        sm: "h-7 px-2 text-xs",
        icon: "h-8 w-8",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export function Button({ className, variant, size, type = "button", ...props }: ButtonProps) {
  return (
    <button type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />
  );
}
