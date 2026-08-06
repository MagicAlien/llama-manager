import { Sidebar } from "@/components/Sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useHashRoute } from "@/lib/useHashRoute";
import type { RouteId } from "@/lib/routes";
import { ApiScreen } from "@/screens/Api";
import { DashboardScreen } from "@/screens/Dashboard";
import { LogsScreen } from "@/screens/Logs";
import { ModelDetailScreen } from "@/screens/ModelDetail";
import { ModelsScreen } from "@/screens/Models";
import { RuntimeScreen } from "@/screens/Runtime";
import { SettingsScreen } from "@/screens/Settings";

const SCREENS: Record<RouteId, () => JSX.Element> = {
  dashboard: DashboardScreen,
  runtime: RuntimeScreen,
  models: ModelsScreen,
  modelDetail: ModelDetailScreen,
  api: ApiScreen,
  logs: LogsScreen,
  settings: SettingsScreen,
};

function App() {
  const route = useHashRoute();
  const ActiveScreen = SCREENS[route];

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
        <Sidebar active={route} />
        <main className="flex-1 overflow-y-auto p-6">
          <ActiveScreen />
        </main>
      </div>
    </TooltipProvider>
  );
}

export default App;
