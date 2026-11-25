import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MarkdownBlock } from "./components/common/MarkdownBlock";
import { QuotaMeter } from "./components/common/Badges";
import { DiffViewer } from "./components/common/DiffViewer";

afterEach(() => {
  cleanup();
});

describe("UI primitives", () => {
  it("renders markdown code fences without highlighting internal @mentions as mentions", () => {
    const md =
      "## Summary\n\nPing @codex outside.\n- Item one\n```python\n@decorator\ndef foo(): pass\n```";
    render(<MarkdownBlock content={md} />);
    expect(screen.getByText("Summary")).toBeTruthy();
    expect(screen.getByText("@codex")).toBeTruthy();
    expect(screen.getByText(/@decorator/)).toBeTruthy();
  });

  it("renders quota meter percentage, exhausted, n/a, and 429 cooldown states", () => {
    const { rerender } = render(<QuotaMeter label="5h" pct={72} />);
    expect(screen.getByText("72%")).toBeTruthy();

    rerender(<QuotaMeter label="Weekly" pct={null} />);
    expect(screen.getByText("n/a")).toBeTruthy();

    rerender(<QuotaMeter label="5h" pct={0} rateLimited />);
    expect(screen.getByText("429 COOLDOWN")).toBeTruthy();
  });

  it("renders unified diffs grouped by file", () => {
    const diff =
      "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-let x = 1;\n+let x = 2;";
    render(<DiffViewer diffText={diff} />);
    expect(screen.getByText("src/main.rs")).toBeTruthy();
    expect(screen.getByText("+1")).toBeTruthy();
    expect(screen.getByText("-1")).toBeTruthy();
  });
});
