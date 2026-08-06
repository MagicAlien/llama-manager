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
  return match ? match.id : DEFAULT_ROUTE_ID;
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
