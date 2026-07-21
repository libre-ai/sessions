import { describe, expect, test } from "bun:test";
import { renderStaticDocument } from "@libre-ai/web-platform";
import { sessionsCockpitDocument } from "../shared/document";
import { COCKPIT_FIXTURE } from "./fixture";

// The read view is static (no client module), so the deterministic static render
// is the document the browser receives without JavaScript.
function renderCockpit(): string {
  return new TextDecoder().decode(renderStaticDocument(sessionsCockpitDocument(COCKPIT_FIXTURE)));
}

describe("sessions cockpit accessible read view", () => {
  test("renders a well-formed HTML document", async () => {
    const html = renderCockpit();
    expect(html).toStartWith("<!doctype html>");
    expect(html).toContain('lang="fr"');
    expect(html).toContain("Libre AI — Sessions");
  });

  test("presents an accessible table with a caption and column headers", async () => {
    const html = renderCockpit();
    expect(html).toContain("<caption>");
    expect(html).toContain('scope="col"');
    expect(html).toContain('scope="row"');
    expect(html).toContain("État");
    expect(html).toContain("Révision");
    expect(html).toContain("Événements");
    // A skip link and a main landmark anchor keyboard navigation.
    expect(html).toContain('href="#sessions"');
    expect(html).toContain('id="sessions"');
  });

  test("conveys the lifecycle as text, never colour alone", async () => {
    const html = renderCockpit();
    // Each cumulative lifecycle stage renders its human label.
    expect(html).toContain("Active");
    expect(html).toContain("Close");
    expect(html).toContain("Exportée");
    expect(html).toContain("Supprimée");
    // No inline colour styling is used to carry meaning.
    expect(html).not.toContain("style=");
  });

  test("lists every fixture session by id", async () => {
    const html = renderCockpit();
    for (const session of COCKPIT_FIXTURE) {
      expect(html).toContain(session.sessionId);
    }
    expect(html).toContain(`${COCKPIT_FIXTURE.length} session(s).`);
  });
});
