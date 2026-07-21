import type { DocumentDescriptor } from "@libre-ai/web-platform";
import type { SessionState } from "../domain/session-event";
import { SessionsCockpit } from "../ui/sessions-cockpit";

// The read-only cockpit is server-rendered and works without JavaScript, so no
// client module is declared; interactivity (command journeys, live regions)
// arrives with a later increment.
export function sessionsCockpitDocument(sessions: readonly SessionState[]): DocumentDescriptor {
  return {
    app: <SessionsCockpit sessions={sessions} />,
    description: "Cockpit humain des sessions de travail collectif sourcé de Libre AI.",
    lang: "fr",
    title: "Libre AI — Sessions",
  };
}
