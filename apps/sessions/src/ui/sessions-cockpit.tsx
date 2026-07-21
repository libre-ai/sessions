// Read-only Sessions cockpit view. Accessibility first (docs/apps/sessions.md
// §Accessibility): an ordered textual/table view; the session lifecycle is
// conveyed as text and never relies on colour. Server-rendered and usable without
// JavaScript — the command journeys (join/contribute/close/…), the live transport
// and audience projections arrive in later increments.

import type { SessionState } from "../domain/session-event";

// The lifecycle flags are cumulative (a session is closed, then exported, then
// deleted); the label is the most-advanced terminal state reached, derived here
// with no colour and no inference beyond the flags themselves.
function lifecycleLabel(session: SessionState): string {
  if (session.deleted) return "Supprimée";
  if (session.exported) return "Exportée";
  if (session.closed) return "Close";
  return "Active";
}

export function SessionsCockpit({ sessions }: { readonly sessions: readonly SessionState[] }) {
  return (
    <>
      <a className="skip-link" href="#sessions">
        Aller à la liste des sessions
      </a>
      <header>
        <h1>Sessions</h1>
        <p>
          Observer les sessions de travail collectif sourcé : cycle de vie, activité et clôture.
          Chaque contribution reste soumise à ses règles d'audience ; l'approbation humaine borne
          chaque résultat partagé.
        </p>
      </header>
      <main id="sessions">
        <h2 id="sessions-heading">Sessions suivies</h2>
        <p>{`${sessions.length} session(s).`}</p>
        <table aria-labelledby="sessions-heading">
          <caption>
            Liste des sessions : identifiant, état du cycle de vie, révision et nombre d'événements.
            L'état est indiqué en toutes lettres.
          </caption>
          <thead>
            <tr>
              <th scope="col">Session</th>
              <th scope="col">État</th>
              <th scope="col">Révision</th>
              <th scope="col">Événements</th>
            </tr>
          </thead>
          <tbody>
            {sessions.map((session) => (
              <tr key={session.sessionId}>
                <th scope="row">{session.sessionId}</th>
                <td>{lifecycleLabel(session)}</td>
                <td>{session.latestRevision}</td>
                <td>{session.eventCount}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </main>
    </>
  );
}
