import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import App from "./App";
import { ROUTES } from "@/lib/routes";
import { strings } from "@/lib/strings";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

// docs/TASKS.md T-005 acceptance: "Seven routes navigable." Proven
// directly rather than asserted: for each of the seven route hashes,
// set window.location.hash (exactly what a real `<a href="#/...">`
// click does) and confirm the matching screen's title renders.
describe("App routing", () => {
  it("navigates to all seven routes via their hash links", () => {
    expect(ROUTES).toHaveLength(7);

    render(<App />);

    for (const route of ROUTES) {
      window.location.hash = route.hash;
      window.dispatchEvent(new HashChangeEvent("hashchange"));

      const expectedTitle = strings.screens[route.id].title;
      expect(screen.getAllByText(expectedTitle).length).toBeGreaterThan(0);
    }
  });

  it("falls back to the dashboard on an unknown hash", () => {
    render(<App />);
    window.location.hash = "#/does-not-exist";
    window.dispatchEvent(new HashChangeEvent("hashchange"));

    expect(screen.getAllByText(strings.screens.dashboard.title).length).toBeGreaterThan(0);
  });
});
