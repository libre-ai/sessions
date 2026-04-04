// Minimal Presto-Matic web client: host creates a session and opens questions;
// participants join by link, answer, and see the leaderboard. The server is the
// authority (Biscuit tokens, server-side timing); this is a thin view over the
// WS protocol.

const $ = (s) => document.querySelector(s);
const show = (id) => ($("#" + id).hidden = false);
const hide = (id) => ($("#" + id).hidden = true);
const log = (m) => ($("#log").textContent += m + "\n");
const postJSON = (path) => fetch(path, { method: "POST" }).then((r) => r.json());

let ws;
let sessionId;
let currentQid;
let isHost = false;

function wsUrl(token, name) {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  let u = `${proto}://${location.host}/ws/${sessionId}?token=${encodeURIComponent(token)}`;
  if (name) u += `&name=${encodeURIComponent(name)}`;
  return u;
}

function connect(token, name) {
  ws = new WebSocket(wsUrl(token, name));
  ws.onopen = () => log("connecté");
  ws.onclose = () => log("déconnecté");
  ws.onmessage = (e) => onMessage(JSON.parse(e.data));
}

function onMessage(m) {
  switch (m.type) {
    case "joined":
      $("#hoststatus").textContent = `${m.participants} participant(s)`;
      break;
    case "question_opened":
      renderQuestion(m.question);
      break;
    case "answer_received":
      log("réponse reçue : " + m.participant_id);
      break;
    case "answers_revealed":
      renderLeaderboard(m);
      break;
    case "breakout_opened":
      renderBreakout(m);
      break;
    case "error":
      log("erreur : " + m.reason);
      break;
  }
}

function submitAnswer(choicesArr, container) {
  ws.send(JSON.stringify({ type: "submit_answer", question_id: currentQid, choices: choicesArr }));
  [...container.children].forEach((x) => (x.disabled = true));
}

function renderQuestion(q) {
  currentQid = q.id;
  hide("leaderboard");
  hide("breakout");
  show("play");
  $("#question").textContent = q.text;
  const choices = $("#choices");
  choices.innerHTML = "";
  const multi = q.kind === "multi";
  const selected = new Set();

  q.choices.forEach((choice, i) => {
    const b = document.createElement("button");
    b.textContent = choice;
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

  if (multi) {
    const validate = document.createElement("button");
    validate.textContent = "Valider";
    validate.onclick = () => submitAnswer([...selected], choices);
    choices.appendChild(validate);
  }
}

function renderLeaderboard(m) {
  show("leaderboard");
  const board = $("#board");
  board.innerHTML = "";
  (m.leaderboard || []).forEach((e) => {
    const li = document.createElement("li");
    li.textContent = `${e.name || e.participant_id} — ${e.score}`;
    board.appendChild(li);
  });

  // The host can open a grounded breakout for any confused section.
  const heatmap = $("#heatmap");
  heatmap.innerHTML = "";
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

async function createSession() {
  const { data } = await postJSON("/sessions");
  isHost = true;
  sessionId = data.session_id;
  $("#code").textContent = data.session_id;
  const a = $("#joinlink");
  a.href = data.join_url;
  a.textContent = location.origin + data.join_url;
  hide("landing");
  show("host");
  connect(data.host_token);
}

async function joinSession() {
  const name = $("#name").value || "anon";
  const { data } = await postJSON(`/sessions/${sessionId}/participants`);
  hide("join");
  connect(data.participant_token, name);
}

function init() {
  const s = new URLSearchParams(location.search).get("s");
  if (s) {
    sessionId = s;
    $("#join-code").textContent = s;
    show("join");
    $("#do-join").onclick = joinSession;
  } else {
    show("landing");
    $("#create").onclick = createSession;
  }
  $("#generate").onclick = () =>
    ws.send(JSON.stringify({ type: "generate_question", query: $("#query").value || "general" }));
  $("#reveal").onclick = () => ws.send(JSON.stringify({ type: "reveal" }));
}

init();
