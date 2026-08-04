import { strings } from "@/lib/strings";

function App() {
  return (
    <main className="flex h-screen flex-col items-center justify-center gap-2 bg-slate-900 text-slate-100">
      <h1 className="text-2xl font-semibold">{strings.appTitle}</h1>
      <p className="text-sm text-slate-400">{strings.scaffoldNotice}</p>
    </main>
  );
}

export default App;
