import type { ComponentPropsWithoutRef, ElementRef } from "react";
import { forwardRef } from "react";
import * as SeparatorPrimitive from "@radix-ui/react-separator";
import { cn } from "@/lib/cn";

// shadcn/ui-style primitive over Radix's Separator (T-005 PR
// Dependencies: `@radix-ui/react-separator` is not on `PLAN.md` §3's
// approved npm list by name — shadcn's own component is a thin
// wrapper around it, per-primitive, the same way every shadcn
// component is. Justified here per `AGENTS.md` §4 / the D-008
// precedent: divides the sidebar header from the nav list and the nav
// list from the footer, with correct `role="separator"` /
// `aria-orientation` semantics a plain `<hr>` doesn't give for a
// vertical divider. Genuinely used in this task, not speculative.
export const Separator = forwardRef<
  ElementRef<typeof SeparatorPrimitive.Root>,
  ComponentPropsWithoutRef<typeof SeparatorPrimitive.Root>
>(({ className, orientation = "horizontal", decorative = true, ...props }, ref) => (
  <SeparatorPrimitive.Root
    ref={ref}
    orientation={orientation}
    decorative={decorative}
    className={cn(
      "shrink-0 bg-border",
      orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
      className,
    )}
    {...props}
  />
));
Separator.displayName = SeparatorPrimitive.Root.displayName;
