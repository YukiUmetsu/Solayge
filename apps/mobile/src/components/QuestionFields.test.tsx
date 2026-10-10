import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ApprovalField } from "@solayge/shared-types";

import { QuestionFields } from "./QuestionFields";

function field(partial: Partial<ApprovalField> & { key: string }): ApprovalField {
  return {
    label: partial.key,
    description: null,
    kind: "string",
    required: false,
    custom: false,
    options: [],
    placeholder: null,
    url: null,
    ...partial,
  };
}

describe("QuestionFields external fields", () => {
  it("renders an https external field as a link", () => {
    render(
      <QuestionFields
        fields={[
          field({ key: "oauth", kind: "external", url: "https://example.test/oauth" }),
        ]}
        answers={{}}
        onChange={() => {}}
        disabled={false}
      />,
    );
    const link = screen.getByRole("link", { name: "https://example.test/oauth" });
    expect(link).toHaveAttribute("href", "https://example.test/oauth");
  });

  it("never turns a javascript: URL from the agent into a clickable link", () => {
    render(
      <QuestionFields
        fields={[field({ key: "evil", kind: "external", url: "javascript:alert(1)" })]}
        answers={{}}
        onChange={() => {}}
        disabled={false}
      />,
    );
    expect(screen.queryByRole("link")).toBeNull();
    // The URL is still shown, just not as an active navigation.
    expect(screen.getByText("javascript:alert(1)")).toBeInTheDocument();
  });
});
