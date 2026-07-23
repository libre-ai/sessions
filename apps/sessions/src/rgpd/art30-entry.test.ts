import { describe, expect, test } from "bun:test";
import { generateArt30Register, validateProcessingActivity } from "@libre-ai/rgpd-kit";
import entry from "../../art30-register.json";

// The Sessions Art. 30 declaration (design §5 step 4) stays contract-valid:
// the JSON file is the source the owner/DPO aggregates, so it must always
// pass the shared validator and render into the register.
describe("apps/sessions/art30-register.json", () => {
  test("passes the shared ProcessingActivity validator", () => {
    const activity = validateProcessingActivity(entry);
    expect(activity.product).toBe("libre-ai/sessions");
    expect(activity.retentionRule).toBe("sessions-content");
    // Restriction and portability are deferred typed refusals, so they are
    // deliberately NOT declared as implemented.
    expect(activity.subjectRightsImplemented).toEqual(["access", "erasure"]);
  });

  test("renders into the generated register", () => {
    const register = generateArt30Register([validateProcessingActivity(entry)]);
    expect(register).toContain("## libre-ai/sessions — Sessions collaborative events");
    expect(register).toContain("- **Retention rule:** sessions-content");
  });
});
