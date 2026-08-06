import type { ComponentPropsWithoutRef, ElementRef } from "react";
import { forwardRef } from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { cn } from "@/lib/cn";

// shadcn/ui-style primitive over Radix's Tooltip (T-005 PR
// Dependencies: `@radix-ui/react-tooltip`, same justification pattern
// as Separator — not on `PLAN.md` §3 by name, added and named here per
// `AGENTS.md` §4). Used by the collapsed sidebar (`Sidebar.tsx`): once
// icons are off the table (T-005 takes no assets or icons from LM
// Studio and this task adds no icon library — `lucide-react` is
// approved but not concretely needed yet), the collapsed rail falls
// back to single-letter badges, and a tooltip is what keeps a
// letter-only nav item legible on hover/focus rather than a guess.
export const TooltipProvider = TooltipPrimitive.Provider;
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export const TooltipContent = forwardRef<
  ElementRef<typeof TooltipPrimitive.Content>,
  ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={cn(
        "z-50 rounded-md border border-border bg-surface px-2 py-1 text-xs text-foreground shadow-md",
        className,
      )}
      {...props}
    />
  </TooltipPrimitive.Portal>
));
TooltipContent.displayName = TooltipPrimitive.Content.displayName;
