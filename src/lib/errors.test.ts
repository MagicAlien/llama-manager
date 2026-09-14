import { describe, expect, it } from "vitest";
import { describeError } from "./errors";

// T-036 — the UI must never render "[object Object]" where a reason belongs.
// Every case below is a real rejection shape from `core/types.rs`'s AppError.
describe("describeError", () => {
  it("returns a string rejection unchanged", () => {
    expect(describeError("boom")).toBe("boom");
  });

  it("reads the message out of the serialized AppError", () => {
    const rejection = {
      kind: "Internal",
      detail: { message: "registration channel is Undetermined; cannot preview preset" },
    };
    expect(describeError(rejection)).toBe(
      "registration channel is Undetermined; cannot preview preset",
    );
  });

  it("describes variants that carry no message", () => {
    // NotFound — `what`
    expect(describeError({ kind: "NotFound", detail: { what: "model model-1" } })).toBe(
      "NotFound — what: model model-1",
    );
    // InvalidPath — two fields
    expect(
      describeError({ kind: "InvalidPath", detail: { path: "E:/x.gguf", reason: "missing" } }),
    ).toBe("InvalidPath — path: E:/x.gguf, reason: missing");
    // Unauthorized — no detail at all
    expect(describeError({ kind: "Unauthorized" })).toBe("Unauthorized");
  });

  it("does not print an object literal for a non-AppError rejection", () => {
    // Tauri's own "command not found" shape: no `kind` field.
    const described = describeError({ message: "command not found" });
    expect(described).not.toBe("[object Object]");
    expect(described).toContain("command not found");
  });

  it("handles a null or undefined rejection", () => {
    expect(describeError(null)).toBe("");
    expect(describeError(undefined)).toBe("");
  });
});
