import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { MarkdownBlock } from "./components/common/MarkdownBlock";
import { QuotaMeter } from "./components/common/Badges";

describe("UI primitives", () => {
  it("renders markdown code fences without highlighting internal @mentions as mentions", () => {
    const md = "Ping @codex outside.\n```python\n@decorator\ndef foo(): pass\n```";
    render(<MarkdownBlock content={md} />);
    expect(screen.getByText("@codex")).toBeTruthy();
    expect(screen.getByText(/@decorator/)).toBeTruthy();
  });

  it("renders quota meter percentage and 429 cooldown state", () => {
    const { rerender } = render(<QuotaMeter label="5h" pct={72} />);
    expect(screen.getByText("72%")).toBeTruthy();
    rerender(<QuotaMeter label="5h" pct={0} rateLimited />);
    expect(screen.getByText("429 COOLDOWN")).toBeTruthy();
  });
});
