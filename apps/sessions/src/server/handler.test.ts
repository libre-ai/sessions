import { describe, expect, test } from "bun:test";
import { createSessionsHandler } from "./handler";

const handler = createSessionsHandler(() => "req_0000000000000000");

describe("sessions cockpit handler", () => {
  test("serves the server-rendered cockpit at /", async () => {
    const response = await handler(new Request("https://sessions.test/"));
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toContain("text/html");
    const html = await response.text();
    expect(html).toContain("Sessions");
    expect(html).toContain("<caption>");
  });

  test("reports health as JSON", async () => {
    const response = await handler(new Request("https://sessions.test/api/health"));
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      service: "libre-ai-sessions",
      status: "ok",
      version: "v1",
    });
  });

  test("an unknown route is not found", async () => {
    const response = await handler(new Request("https://sessions.test/nope"));
    expect(response.status).toBe(404);
  });
});
