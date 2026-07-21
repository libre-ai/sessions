import { createRequestHandler, renderSsrDocument } from "@libre-ai/web-platform";
import { sessionsCockpitDocument } from "../shared/document";
import { COCKPIT_FIXTURE } from "../ui/fixture";

// The Sessions cockpit request handler. The read view is server-rendered from a
// contract fixture (the spec's runtime boundary: no real session or transport
// integration until a bounded work package and conformance review are approved).
// No client assets are served — the view works without JavaScript.
export function createSessionsHandler(
  requestId: (request: Request) => string = () => `req_${crypto.randomUUID().replaceAll("-", "")}`,
): (request: Request) => Promise<Response> {
  return createRequestHandler({
    requestId,
    routes: {
      "/": () => renderSsrDocument(sessionsCockpitDocument(COCKPIT_FIXTURE)),
      "/api/health": () =>
        Response.json({ service: "libre-ai-sessions", status: "ok", version: "v1" }),
    },
  });
}
