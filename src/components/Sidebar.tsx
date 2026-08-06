import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/cn";
import { ROUTES, type RouteId } from "@/lib/routes";
import { strings } from "@/lib/strings";

interface SidebarProps {
  active: RouteId;
}

// App shell sidebar (docs/TASKS.md T-005): seven navigable, real
// `<a href="#/...">` links (see useHashRoute.ts) plus a collapse
// toggle. Collapsed state is local UI chrome, not the application
// state docs/WORKFLOW.md §3 scopes this task away from. Collapsed rail
// falls back to single-letter badges + tooltips instead of icons —
// see tooltip.tsx for why.
export function Sidebar({ active }: SidebarProps) {
  const [collapsed, setCollapsed] = useState(false);

  return (
    <aside
      className={cn(
        "flex h-full flex-col border-r border-border bg-surface transition-[width] duration-150",
        collapsed ? "w-16" : "w-56",
      )}
    >
      <div className="flex items-center justify-between px-3 py-4">
        {!collapsed && (
          <span className="truncate text-sm font-semibold text-foreground">
            {strings.appTitle}
          </span>
        )}
        <Button
          variant="ghost"
          size="icon"
          onClick={() => setCollapsed((value) => !value)}
          aria-label={collapsed ? strings.sidebar.expand : strings.sidebar.collapse}
        >
          <span aria-hidden="true">
            {collapsed ? strings.sidebar.expandGlyph : strings.sidebar.collapseGlyph}
          </span>
        </Button>
      </div>

      <Separator />

      <nav className="flex flex-1 flex-col gap-1 p-2" aria-label={strings.sidebar.navLabel}>
        {ROUTES.map((route) => {
          const isActive = route.id === active;
          const item = strings.screens[route.id];

          const link = (
            <a
              href={route.hash}
              aria-current={isActive ? "page" : undefined}
              className={cn(
                "flex items-center rounded-md px-3 py-2 text-sm transition-colors",
                collapsed ? "justify-center" : "gap-2",
                isActive
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-surface-hover hover:text-foreground",
              )}
            >
              <span
                aria-hidden={collapsed ? undefined : "true"}
                className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-xs font-semibold"
              >
                {item.initial}
              </span>
              {!collapsed && <span>{item.navLabel}</span>}
            </a>
          );

          if (!collapsed) {
            return <div key={route.id}>{link}</div>;
          }

          return (
            <Tooltip key={route.id}>
              <TooltipTrigger asChild>{link}</TooltipTrigger>
              <TooltipContent side="right">{item.navLabel}</TooltipContent>
            </Tooltip>
          );
        })}
      </nav>
    </aside>
  );
}
