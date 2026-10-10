import type { InboxFilter } from "./client/selectors";
import type { TabId } from "./tabs";

/** The mobile information architecture routes. */
export type Route =
  | { name: "sessions" }
  | { name: "session"; sessionId: string }
  | { name: "inbox"; filter: InboxFilter }
  | { name: "approval"; approvalId: string }
  | { name: "projects" }
  | { name: "project"; projectId: string }
  | { name: "settings" };

/** Which bottom tab a route belongs to (settings belongs to none). */
export function routeTab(route: Route): TabId | null {
  switch (route.name) {
    case "sessions":
    case "session":
      return "sessions";
    case "inbox":
    case "approval":
      return "inbox";
    case "projects":
    case "project":
      return "projects";
    case "settings":
      return null;
  }
}

/** Whether a route is a root destination (no back control). */
export function isRootRoute(route: Route): boolean {
  return route.name === "sessions" || route.name === "inbox" || route.name === "projects";
}
