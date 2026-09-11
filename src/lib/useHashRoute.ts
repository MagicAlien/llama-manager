import { useEffect, useState } from "react";
import { DEFAULT_ROUTE_ID, ROUTES, type RouteId } from "./routes";

// Deliberately not a routing library: no router package is on
// `PLAN.md` §3's approved list, and this task adds no new dependency
// beyond the justified `@radix-ui/react-*` primitives (see PR
// Dependencies). Seven static, non-functional screens don't need one
// — real `<a href="#/...">` links plus the browser's own `hashchange`
// event are enough to make every route independently navigable
// (including via the address bar), and this is local UI state, not
// the application state management docs/WORKFLOW.md §3 scopes T-005
// away from.
function parseHash(hash: string): RouteId {
  const match = ROUTES.find((route) => route.hash === hash);
  if (match) return match.id;
  // The model-detail route carries a model id in the hash
  // (`#/models/detail/<id>`) — match it by prefix, not exact equality,
  // so the id never has to be a known route.
  if (hash.startsWith("#/models/detail/")) return "modelDetail";
  return DEFAULT_ROUTE_ID;
}

// Read the model id out of `#/models/detail/<id>`, or null when the
// route is reached without one (e.g. via the sidebar link).
export function getModelIdFromHash(hash: string): string | null {
  const prefix = "#/models/detail/";
  if (!hash.startsWith(prefix)) return null;
  const id = hash.slice(prefix.length);
  return id.length > 0 ? id : null;
}

export function useHashRoute(): RouteId {
  const [route, setRoute] = useState<RouteId>(() =>
    typeof window === "undefined" ? DEFAULT_ROUTE_ID : parseHash(window.location.hash),
  );

  useEffect(() => {
    const onHashChange = () => setRoute(parseHash(window.location.hash));
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  return route;
}
