// Presto-Matic web client: host creates a session and opens questions;
// participants join by link, answer, and see the leaderboard. The server is the
// authority (Biscuit tokens, server-side timing); this is a thin view over the
// WS protocol.
//
// Features:
// - Robust error handling and user feedback
// - Automatic WS reconnection with exponential backoff
// - Loading states and disabled inputs during API calls
// - Validation of user inputs
// - Late-join support (question snapshot on connect)

const $ = (s) => document.querySelector(s);
const $$ = (s) => document.querySelectorAll(s);
const show = (id) => ($("#" + id).hidden = false);
const hide = (id) => ($("#" + id).hidden = true);
const log = (m) => ($("#log").textContent += m + "\n");
const setLoading = (enabled) => {
  $$("button, input").forEach((el) => {
    el.disabled = enabled;
  });
  const loader = $("#loader");
  if (loader) loader.hidden = !enabled;
};
const postJSON = async (path) => {
  try {
    const r = await fetch(path, { method: "POST" });
    if (!r.ok) {
      const msg = r.status === 404 ? "Session introuvable (code expiré?)" : `Erreur HTTP ${r.status}`;
      throw new Error(msg);
    }
    return await r.json();
  } catch (e) {
    log(`erreur réseau: ${e.message}`);
    throw e;
  }
};

let ws;
let sessionId;
let currentQid;
let isHost = false;
let currentName = "";
let reconnectAttempt = 0;
let maxReconnectAttempts = 5;
let reconnectDelay = 1000;  // ms, exponential backoff

function wsUrl(token) {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  let u = `${proto}://${location.host}/ws/${sessionId}?token=${encodeURIComponent(token)}`;
  if (currentName) u += `&name=${encodeURIComponent(currentName)}`;
  return u;
}

function connect(token, name) {
  if (name) currentName = name;

  if (ws) {
    try {
      ws.close();
    } catch (e) {}
  }

  log("connexion en cours...");
  ws = new WebSocket(wsUrl(token));

  ws.onopen = () => {
    log("connecté");
    reconnectAttempt = 0;
    reconnectDelay = 1000;
    setLoading(false);
  };

  ws.onclose = () => {
    log("déconnecté");
    // Attempt reconnection with exponential backoff.
    if (reconnectAttempt < maxReconnectAttempts) {
      reconnectAttempt++;
      log(`reconnexion dans ${reconnectDelay}ms...`);
      setTimeout(() => {
        if (token && sessionId) {
          connect(token, currentName);
          reconnectDelay = Math.min(reconnectDelay * 2, 10000);  // cap at 10s
        }
      }, reconnectDelay);
    } else {
      log("reconnexion échouée, rechargez la page.");
      setLoading(true);
    }
  };

  ws.onerror = (e) => {
    log(`erreur WS: ${e.message || "connexion perdue"}`);
  };

  ws.onmessage = (e) => {
    try {
      const msg = JSON.parse(e.data);
      onMessage(msg);
    } catch (e) {
      log(`erreur parsing: ${e.message}`);
    }
  };
}

function onMessage(m) {
  if (!m || !m.type) {
    log("message malformé reçu");
    return;
  }

  switch (m.type) {
    case "joined":
      if (m.participants !== undefined) {
        $("#hoststatus").textContent = `${m.participants} participant(s)`;
      }
      break;
    case "question_opened":
      if (m.question) {
        renderQuestion(m.question);
      } else {
        log("erreur: question vide");
      }
      break;
    case "answer_received":
      if (m.participant_id) {
        log("réponse reçue : " + m.participant_id);
      }
      break;
    case "answers_revealed":
      if (m.leaderboard !== undefined) {
        renderLeaderboard(m);
      } else {
        log("erreur: leaderboard vide");
      }
      break;
    case "breakout_opened":
      if (m.section_id && m.explanation) {
        renderBreakout(m);
      } else {
        log("erreur: breakout incomplet");
      }
      break;
    case "flashcards_ready":
      if (m.cards !== undefined) {
        renderFlashcards(m);
      } else {
        log("erreur: flashcards non disponibles");
      }
      break;
    case "error":
      log("erreur serveur : " + (m.reason || "inconnu"));
      break;
    case "pong":
      // Heartbeat response; do nothing.
      break;
    default:
      log(`type de message inconnu: ${m.type}`);
  }
}

function submitAnswer(choicesArr, container) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    log("erreur: pas de connexion (reconnexion en cours?)");
    return;
  }

  if (!currentQid || !choicesArr || choicesArr.length === 0) {
    log("erreur: question ou choix invalide");
    return;
  }

  try {
    ws.send(JSON.stringify({ type: "submit_answer", question_id: currentQid, choices: choicesArr }));
    [...container.children].forEach((x) => (x.disabled = true));
    log("réponse envoyée");
  } catch (e) {
    log(`erreur envoi: ${e.message}`);
  }
}

function validationLabel(status) {
  switch (status) {
    case "verified":
      return "validée par grounding-verifier";
    case "fixture":
      return "fixture de démonstration";
    default:
      return "non validée";
  }
}

function renderQuestion(q) {
  if (!q || !q.id) {
    log("erreur: question invalide");
    return;
  }

  currentQid = q.id;
  hide("leaderboard");
  hide("breakout");
  show("play");

  // Display question text.
  const questionEl = $("#question");
  questionEl.textContent = q.text || "(pas de texte)";

  // Display grounding information.
  const grounding = q.grounding || {};
  const count = grounding.citation_count || 0;
  const groundingEl = $("#grounding");
  groundingEl.textContent = grounding.grounded
    ? `Question sourcée (${validationLabel(grounding.validation_status)}, ${count} citation(s), refs privées)`
    : "Question non validée par citation.";

  // Render choices.
  const choices = $("#choices");
  choices.replaceChildren();
  const multi = q.kind === "multi";
  const selected = new Set();

  if (!q.choices || q.choices.length === 0) {
    log("erreur: aucun choix disponible");
    return;
  }

  q.choices.forEach((choice, i) => {
    const b = document.createElement("button");
    b.textContent = choice;
    b.dataset.index = i;
    b.onclick = () => {
      if (multi) {
        if (selected.has(i)) {
          selected.delete(i);
          b.textContent = choice;
        } else {
          selected.add(i);
          b.textContent = "☑ " + choice;
        }
      } else {
        submitAnswer([i], choices);
        b.textContent = "✓ " + choice;
      }
    };
    choices.appendChild(b);
  });

  // Add validation button for multi-select.
  if (multi) {
    const validate = document.createElement("button");
    validate.textContent = "Valider";
    validate.onclick = () => {
      if (selected.size === 0) {
        log("choisissez au moins une réponse");
        return;
      }
      submitAnswer([...selected], choices);
    };
    choices.appendChild(validate);
  }
}

function renderLeaderboard(m) {
  show("leaderboard");
  show("flashcards"); // participants can now request a spaced-repetition deck
  const board = $("#board");
  board.replaceChildren();
  (m.leaderboard || []).forEach((e) => {
    const li = document.createElement("li");
    li.textContent = `${e.name || e.participant_id} — ${e.score}`;
    board.appendChild(li);
  });

  // The host can open a grounded breakout for any confused section.
  const heatmap = $("#heatmap");
  heatmap.replaceChildren();
  if (isHost && m.heatmap) {
    Object.entries(m.heatmap).forEach(([section, confusion]) => {
      const b = document.createElement("button");
      b.textContent = `Clarifier ${section} (confusion ${Math.round(confusion * 100)}%)`;
      b.onclick = () => ws.send(JSON.stringify({ type: "breakout", section_id: section }));
      heatmap.appendChild(b);
    });
  }
}

function renderBreakout(m) {
  show("breakout");
  $("#breakout-section").textContent = m.section_id;
  $("#breakout-text").textContent = m.explanation;
}

function renderFlashcards(m) {
  show("flashcards");
  const deck = $("#deck");
  deck.replaceChildren();
  if (!m.cards || m.cards.length === 0) {
    const li = document.createElement("li");
    li.textContent = "Aucune section faible — bien joué.";
    deck.appendChild(li);
    return;
  }
  m.cards.forEach((c) => {
    const li = document.createElement("li");
    li.textContent = `${c.front} — ${c.back} (${c.section_id})`;
    deck.appendChild(li);
  });
}

async function createSession() {
  setLoading(true);
  try {
    const { data } = await postJSON("/sessions");
    if (!data || !data.session_id || !data.host_token) {
      throw new Error("réponse invalide du serveur");
    }
    isHost = true;
    sessionId = data.session_id;
    $("#code").textContent = data.session_id;
    const secureJoin = data.secure_join_url || data.join_url;
    const secure = $("#secure-joinlink");
    secure.href = secureJoin;
    secure.textContent = location.origin + secureJoin;
    const a = $("#joinlink");
    a.href = data.join_url;
    a.textContent = location.origin + data.join_url;
    hide("landing");
    show("host");
    connect(data.host_token);
  } catch (e) {
    setLoading(false);
    log(`création échouée: ${e.message}`);
  }
}

async function joinSession() {
  const name = $("#name").value.trim();
  if (!name) {
    log("entrez un nom");
    return;
  }
  if (!sessionId) {
    log("erreur: session ID manquant");
    return;
  }

  setLoading(true);
  try {
    const { data } = await postJSON(`/sessions/${sessionId}/participants`);
    if (!data || !data.participant_token) {
      throw new Error("réponse invalide du serveur");
    }
    hide("join");
    connect(data.participant_token, name);
  } catch (e) {
    setLoading(false);
    log(`join échoué: ${e.message}`);
  }
}

function init() {
  // Initialize event handlers.
  const s = new URLSearchParams(location.search).get("s");
  if (s) {
    sessionId = s.trim().toUpperCase();  // Normalize code.
    $("#join-code").textContent = sessionId;
    show("join");
    $("#do-join").onclick = joinSession;
    // Focus name input for quick interaction.
    setTimeout(() => $("#name").focus(), 100);
  } else {
    show("landing");
    $("#create").onclick = createSession;
  }

  // Host controls.
  $("#generate").onclick = () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      log("erreur: pas de connexion");
      return;
    }
    const query = $("#query").value.trim() || "general";
    try {
      ws.send(JSON.stringify({ type: "generate_question", query }));
    } catch (e) {
      log(`erreur envoi: ${e.message}`);
    }
  };

  $("#reveal").onclick = () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      log("erreur: pas de connexion");
      return;
    }
    try {
      ws.send(JSON.stringify({ type: "reveal" }));
    } catch (e) {
      log(`erreur envoi: ${e.message}`);
    }
  };

  // Participant controls.
  $("#get-flashcards").onclick = () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      log("erreur: pas de connexion");
      return;
    }
    try {
      ws.send(JSON.stringify({ type: "flashcards" }));
    } catch (e) {
      log(`erreur envoi: ${e.message}`);
    }
  };

  // Heartbeat (optional but good for detecting stale connections).
  setInterval(() => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      try {
        ws.send(JSON.stringify({ type: "ping" }));
      } catch (e) {
        // Ignore; connection will close and reconnect.
      }
    }
  }, 30000);
}

init();
