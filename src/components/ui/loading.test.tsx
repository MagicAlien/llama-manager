import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Loading } from "./loading";

afterEach(() => {
  cleanup();
});

describe("Loading", () => {
  it("renders the required label as visible text", () => {
    render(<Loading label="Checking runtime status" />);
    expect(screen.getByText("Checking runtime status")).toBeTruthy();
  });

  it("exposes role=\"status\" so assistive tech announces the label", () => {
    render(<Loading label="Loading models" />);
    expect(screen.getByRole("status")).toBeTruthy();
  });
});

// Type-level assertion (docs/TASKS.md T-005: "the loading component's
// label prop is required — a bare spinner is a TypeScript error,
// asserted by a type-level test"). vitest transforms this file with
// esbuild, which strips types without checking them, so the two it()
// blocks above cannot catch a type regression, only a runtime one.
// The assertion below belongs to tsc: tsconfig.json's "include":
// ["src"] means every file under src/, including this one, is
// type-checked as part of `npm run build`. This function is never
// called; the directive comment on the next line only needs to be
// seen by tsc, not executed at runtime. It is exported so
// `noUnusedLocals` doesn't itself flag it as dead code.
//
// NOTE for future edits: the suppression comment below must be the
// first line directly above the statement it applies to, with nothing
// else between — TypeScript matches it structurally, not by intent,
// so re-flowing this paragraph so a *different* line starts with the
// directive's own name would silently turn that line into a second,
// unintended directive.
export function typeTest_labelIsRequired() {
  // @ts-expect-error -- `label` is required; a bare <Loading /> must not compile.
  return <Loading />;
}
