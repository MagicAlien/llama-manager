import { strings } from "@/lib/strings";

// Empty, navigable route (docs/TASKS.md T-005). No IPC, no state, no
// data — purely a scaffolded destination (docs/WORKFLOW.md §3 scope
// discipline).
export function SettingsScreen() {
  const copy = strings.screens.settings;

  return (
    <section className="flex flex-col gap-2">
      <h1 className="text-xl font-semibold text-foreground">{copy.title}</h1>
      <p className="text-sm text-muted-foreground">{copy.description}</p>
      <p className="text-xs text-muted-foreground">{strings.emptyScreenNote}</p>
    </section>
  );
}
