import type { ApprovalSummary } from "@solayge/shared-types";

/**
 * Plain-language explanations for an approval. These make the *risk boundary*
 * explicit: what a decision affects, its limits, and whether it can change
 * files or only read them (mobile spec section 7.2 D).
 */

/** Whether the request can change local state. */
export function classification(approval: ApprovalSummary): {
  label: string;
  tone: "neutral" | "warning";
} {
  return approval.read_only
    ? { label: "Read-only", tone: "neutral" }
    : { label: "Can modify files", tone: "warning" };
}

/** The exact operation, never a paraphrase, when one is supplied. */
export function exactOperation(approval: ApprovalSummary): string | null {
  if (approval.command && approval.command.trim().length > 0) return approval.command;
  return null;
}

/** What the decision applies to. */
export function affectedScope(approval: ApprovalSummary): string {
  if (approval.kind === "question") {
    return "This answer applies to this question in this session only.";
  }
  return "This decision applies to this single request in this session only.";
}

/**
 * The limitations of a decision. Always states the scope; adds the read/write
 * boundary so a permission is never mistaken for a blanket grant.
 */
export function limitations(approval: ApprovalSummary): string {
  if (approval.kind === "question") {
    return "Answering a question is not authorization. It cannot execute code or approve any action.";
  }
  if (approval.read_only) {
    return "Approving allows this one read-only operation. It does not grant ongoing access.";
  }
  return "Approving allows this one operation. It does not approve other commands, now or later.";
}

/** A short risk label. */
export function riskLabel(risk: ApprovalSummary["risk"]): string {
  switch (risk) {
    case "low":
      return "Low impact";
    case "medium":
      return "Medium impact";
    case "high":
      return "High impact";
    default:
      return "Unknown impact";
  }
}

/** Whether this decision warrants an extra confirmation step. */
export function requiresConfirmation(approval: ApprovalSummary): boolean {
  return approval.kind === "permission" && approval.risk === "high";
}
