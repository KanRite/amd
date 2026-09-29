// No @tauri-apps/api package on purpose (ticket 078: no npm/bun build
// step for this spike) — `withGlobalTauri` in tauri.conf.json injects
// window.__TAURI__ instead.
const { invoke } = window.__TAURI__.core;

function cardEl(card) {
  const el = document.createElement("article");
  el.className = "card";

  const id = document.createElement("span");
  id.className = "card-id";
  id.textContent = card.id;
  el.appendChild(id);

  const title = document.createElement("h3");
  title.textContent = card.title;
  el.appendChild(title);

  if (card.points || card.assignee) {
    const meta = document.createElement("div");
    meta.className = "card-meta";
    if (card.points) meta.appendChild(Object.assign(document.createElement("span"), { textContent: `${card.points} pts` }));
    if (card.assignee) meta.appendChild(Object.assign(document.createElement("span"), { textContent: card.assignee }));
    el.appendChild(meta);
  }

  if (card.tags.length > 0) {
    const tags = document.createElement("div");
    tags.className = "card-tags";
    for (const tag of card.tags) {
      const t = document.createElement("span");
      t.className = "tag";
      t.textContent = tag;
      tags.appendChild(t);
    }
    el.appendChild(tags);
  }

  return el;
}

function columnEl(column) {
  const el = document.createElement("section");
  el.className = "column";

  const heading = document.createElement("h2");
  heading.textContent = `${column.name.toUpperCase()} (${column.cards.length})`;
  el.appendChild(heading);

  if (column.cards.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "(empty)";
    el.appendChild(empty);
  } else {
    for (const card of column.cards) {
      el.appendChild(cardEl(card));
    }
  }

  return el;
}

async function render() {
  const status = document.getElementById("status");
  try {
    const board = await invoke("board");
    document.getElementById("project-name").textContent = board.project;
    const boardEl = document.getElementById("board");
    boardEl.innerHTML = "";
    for (const column of board.columns) {
      boardEl.appendChild(columnEl(column));
    }
  } catch (err) {
    status.textContent = `Error: ${err}`;
    status.className = "error";
    document.getElementById("board").appendChild(status);
  }
}

render();
