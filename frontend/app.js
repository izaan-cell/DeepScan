// DeepScan frontend logic.
//
// Deliberately holds NO client-side persistence — no localStorage,
// sessionStorage, or IndexedDB. Every query (text, code, or a dropped
// image's raw bytes) is sent straight to the local Rust engine's JSON-HTTP
// shim (see docs/ARCHITECTURE.md #4) and lives there, not in the browser.

// Same origin works when the engine itself serves this page (Local mode,
// the normal case). When this page is instead served from Render (Cloud
// mode — see docs/ARCHITECTURE.md), /api/status there reports mode:
// "cloud", and we fall back to a locally-installed engine on the visitor's
// own machine at the fixed dev/default HTTP port.
const LOCAL_FALLBACK_BASE = "http://127.0.0.1:51424";
let ENGINE_BASE = "";

async function resolveEngineBase() {
  try {
    const res = await fetch(`${ENGINE_BASE}/api/status`);
    const data = await res.json();
    if (data.mode === "cloud") {
      ENGINE_BASE = LOCAL_FALLBACK_BASE;
    }
  } catch {
    ENGINE_BASE = LOCAL_FALLBACK_BASE;
  }
}

const state = {
  scope: "all",
};

const els = {
  queryInput: document.getElementById("queryInput"),
  dropTarget: document.getElementById("dropTarget"),
  dropLabel: document.getElementById("dropLabel"),
  fileInput: document.getElementById("fileInput"),
  scopeSelector: document.getElementById("scopeSelector"),
  statusDot: document.getElementById("statusDot"),
  statusText: document.getElementById("statusText"),
  resultsCanvas: document.getElementById("resultsCanvas"),
};

// ---------- Scope selector ----------

els.scopeSelector.addEventListener("click", (e) => {
  const chip = e.target.closest(".chip");
  if (!chip) return;
  document.querySelectorAll(".chip").forEach((c) => c.classList.remove("is-active"));
  chip.classList.add("is-active");
  state.scope = chip.dataset.scope;
});

// ---------- Text / code query ----------

let debounceTimer = null;
els.queryInput.addEventListener("input", () => {
  autoGrow(els.queryInput);
  clearTimeout(debounceTimer);
  const text = els.queryInput.value.trim();
  if (!text) {
    renderEmpty();
    return;
  }
  debounceTimer = setTimeout(() => runTextSearch(text), 300);
});

function autoGrow(textarea) {
  textarea.style.height = "auto";
  textarea.style.height = textarea.scrollHeight + "px";
}

async function runTextSearch(text) {
  await search({ text_query: text, scope: state.scope });
}

// ---------- Image drop / picker ----------

["dragenter", "dragover"].forEach((evt) =>
  els.dropTarget.addEventListener(evt, (e) => {
    e.preventDefault();
    els.dropTarget.classList.add("is-dragover");
  })
);

["dragleave", "drop"].forEach((evt) =>
  els.dropTarget.addEventListener(evt, (e) => {
    e.preventDefault();
    els.dropTarget.classList.remove("is-dragover");
  })
);

els.dropTarget.addEventListener("drop", (e) => {
  const file = e.dataTransfer.files?.[0];
  if (file) handleImageFile(file);
});

els.dropTarget.addEventListener("click", () => els.fileInput.click());
els.fileInput.addEventListener("change", () => {
  const file = els.fileInput.files?.[0];
  if (file) handleImageFile(file);
});

async function handleImageFile(file) {
  els.dropLabel.textContent = `searching by “${file.name}”…`;
  const bytes = new Uint8Array(await file.arrayBuffer());
  await search({ image_query_bytes: Array.from(bytes), scope: state.scope });
  els.dropLabel.textContent = "drag an image or icon here";
}

// ---------- Search + results ----------

async function search(payload) {
  try {
    const res = await fetch(`${ENGINE_BASE}/api/search`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error(`engine returned ${res.status}`);
    const data = await res.json();
    renderResults(data.results ?? []);
  } catch (err) {
    console.error("[DeepScan] search failed:", err);
    // Show the real underlying error on-screen, not just a generic
    // message — there's no way to open dev tools quickly on this native
    // window, so this is the only way to actually see what broke.
    setStatus(false, `search failed: ${err.name || "Error"}: ${err.message || err} (base="${ENGINE_BASE}")`);
  }
}

function renderResults(results) {
  els.resultsCanvas.innerHTML = "";
  if (results.length === 0) {
    renderEmpty();
    return;
  }
  for (const r of results) {
    const card = document.createElement("article");
    card.className = "result-card";
    card.innerHTML = `
      <div class="file-category">${escapeHtml(r.file.category)}</div>
      <div class="file-name">${escapeHtml(basename(r.file.path))}</div>
      <div class="file-path">${escapeHtml(r.file.path)}</div>
      <button class="reveal-action">reveal in ${platformFileManagerName()}</button>
    `;
    card.querySelector(".reveal-action").addEventListener("click", (e) => {
      e.stopPropagation();
      revealInOs(r.file.path);
    });
    els.resultsCanvas.appendChild(card);
  }
}

function renderEmpty() {
  els.resultsCanvas.innerHTML = `<p class="empty-state">Results will appear here once you search.</p>`;
}

async function revealInOs(path) {
  try {
    await fetch(`${ENGINE_BASE}/api/reveal`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
  } catch (err) {
    console.error("[DeepScan] reveal failed:", err);
  }
}

// ---------- Index status monitor ----------

async function pollStatus() {
  try {
    const res = await fetch(`${ENGINE_BASE}/api/status`);
    if (!res.ok) throw new Error(String(res.status));
    const s = await res.json();
    const scanning = s.is_scanning ? " · scanning…" : "";
    setStatus(s.db_healthy, `${s.total_indexed_files.toLocaleString()} files indexed${scanning}`);
  } catch (err) {
    setStatus(false, `engine unreachable: ${err.name || "Error"}: ${err.message || err} (base="${ENGINE_BASE}")`);
  } finally {
    setTimeout(pollStatus, 4000);
  }
}

function setStatus(healthy, text) {
  els.statusDot.classList.toggle("is-healthy", !!healthy);
  els.statusText.textContent = text;
}

// ---------- helpers ----------

async function init() {
  await resolveEngineBase();
  pollStatus();
}

function basename(path) {
  return path.split(/[\\/]/).pop();
}

function platformFileManagerName() {
  return navigator.platform.toLowerCase().includes("win") ? "Explorer" : "Finder";
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str ?? "";
  return div.innerHTML;
}

init();
