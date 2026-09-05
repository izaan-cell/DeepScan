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
  sendButton: document.getElementById("sendButton"),
  imagePreview: document.getElementById("imagePreview"),
  previewImg: document.getElementById("previewImg"),
  clearImageButton: document.getElementById("clearImageButton"),
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

// Enter inserts a newline (the textarea's own default behavior — nothing
// to wire up) so a query can span multiple lines, e.g. a pasted code
// block. Search only ever runs when Send is actually clicked.
els.queryInput.addEventListener("input", () => {
  autoGrow(els.queryInput);
  const hasText = !!els.queryInput.value.trim();
  els.sendButton.hidden = !hasText;
  if (!hasText) {
    renderEmpty();
  }
});

els.sendButton.addEventListener("click", () => {
  const text = els.queryInput.value.trim();
  if (text) runTextSearch(text);
});

function autoGrow(textarea) {
  textarea.style.height = "auto";
  textarea.style.height = textarea.scrollHeight + "px";
}

async function runTextSearch(text) {
  await search({ text_query: text, scope: state.scope }, text);
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

els.dropTarget.addEventListener("click", (e) => {
  if (e.target === els.clearImageButton) return;
  els.fileInput.click();
});
els.fileInput.addEventListener("change", () => {
  const file = els.fileInput.files?.[0];
  if (file) handleImageFile(file);
});

els.clearImageButton.addEventListener("click", (e) => {
  e.stopPropagation();
  clearImagePreview();
  renderEmpty();
});

// A dropped .app (or any other folder) isn't a real single file — the
// browser hands over a File object for it, but reading its bytes either
// throws or silently returns nothing useful. Without a guard here, that
// left the UI stuck on "searching by X…" forever with no error shown,
// since nothing ever caught it.
const MAX_IMAGE_QUERY_BYTES = 25 * 1024 * 1024;

// Tracks the object URL backing the visible preview so it can be revoked
// (freeing the memory) whenever it's replaced or cleared — object URLs
// otherwise just leak for the life of the page.
let previewObjectUrl = null;

function showImagePreview(file) {
  if (previewObjectUrl) URL.revokeObjectURL(previewObjectUrl);
  previewObjectUrl = URL.createObjectURL(file);
  els.previewImg.src = previewObjectUrl;
  els.imagePreview.hidden = false;
  els.dropLabel.hidden = true;
}

function clearImagePreview() {
  if (previewObjectUrl) {
    URL.revokeObjectURL(previewObjectUrl);
    previewObjectUrl = null;
  }
  els.previewImg.src = "";
  els.imagePreview.hidden = true;
  els.dropLabel.hidden = false;
  els.dropLabel.textContent = "drag an image or icon here";
  els.fileInput.value = "";
}

async function handleImageFile(file) {
  try {
    if (!file.type.startsWith("image/")) {
      throw new Error(`"${file.name}" isn't an image DeepScan can search by`);
    }
    if (file.size > MAX_IMAGE_QUERY_BYTES) {
      throw new Error(`"${file.name}" is too large to search by (${Math.round(file.size / 1024 / 1024)}MB)`);
    }
    showImagePreview(file);
    const bytes = new Uint8Array(await file.arrayBuffer());
    await search({ image_query_bytes: Array.from(bytes), scope: state.scope }, file.name);
  } catch (err) {
    console.error("[DeepScan] image query failed:", err);
    setStatus(false, `image query failed: ${err.name || "Error"}: ${err.message || err}`, true);
  }
}

// ---------- Search + results ----------

async function search(payload, queryLabel) {
  try {
    const res = await fetch(`${ENGINE_BASE}/api/search`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error(`engine returned ${res.status}`);
    const data = await res.json();
    renderResults(data.results ?? [], queryLabel);
  } catch (err) {
    console.error("[DeepScan] search failed:", err);
    // Show the real underlying error on-screen, not just a generic
    // message — there's no way to open dev tools quickly on this native
    // window, so this is the only way to actually see what broke.
    setStatus(false, `search failed: ${err.name || "Error"}: ${err.message || err} (base="${ENGINE_BASE}")`, true);
  }
}

function renderResults(results, queryLabel) {
  els.resultsCanvas.innerHTML = "";
  if (results.length === 0) {
    renderNoResults(queryLabel);
    return;
  }
  for (const r of results) {
    const card = document.createElement("article");
    card.className = "result-card";
    card.innerHTML = `
      <div class="file-category">${escapeHtml(r.category)}</div>
      ${previewHtml(r)}
      <div class="file-name">${escapeHtml(basename(r.path))}</div>
      <div class="file-path">${escapeHtml(r.path)}</div>
      <button class="reveal-action">reveal in ${platformFileManagerName()}</button>
    `;
    card.querySelector(".reveal-action").addEventListener("click", (e) => {
      e.stopPropagation();
      revealInOs(r.path);
    });
    els.resultsCanvas.appendChild(card);
  }
}

// A result card should show enough of the actual file to recognize it at a
// glance without opening it: a real thumbnail for images (fetched from the
// engine's own indexed-files-only /api/thumbnail — see http.rs), or the
// text/code snippet the engine already extracted and returned alongside
// the search result for everything else.
function previewHtml(r) {
  if (r.category === "image") {
    const src = `${ENGINE_BASE}/api/thumbnail?path=${encodeURIComponent(r.path)}`;
    return `<img class="result-thumb" src="${src}" alt="" loading="lazy" onerror="this.remove()">`;
  }
  if (r.snippet) {
    return `<pre class="result-snippet">${escapeHtml(r.snippet)}</pre>`;
  }
  return "";
}

// Distinct from renderEmpty() (the pristine "haven't searched yet" state)
// — this is what a search that actually ran and found nothing shows, and
// it names the query so it's clear the search happened at all rather than
// looking identical to never having searched.
function renderNoResults(queryLabel) {
  const label = queryLabel ? ` for "${escapeHtml(queryLabel)}"` : "";
  els.resultsCanvas.innerHTML = `<p class="empty-state">No results found${label}.</p>`;
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

// pollStatus runs on its own 4s timer independent of anything the user just
// did — without this, an error message shown by search()/handleImageFile()
// (the only way to actually read a real error, since there's no dev tools
// on this native window) could get silently overwritten by the very next
// poll tick, sometimes under a second later depending on where that timer
// happened to be. errorUntil holds the ticker on the error text for a fixed
// window so it's actually readable (and copyable — see the Edit menu in
// DeepScanWindow.swift) before status updates resume.
let errorUntil = 0;

async function pollStatus() {
  try {
    const res = await fetch(`${ENGINE_BASE}/api/status`);
    if (!res.ok) throw new Error(String(res.status));
    const s = await res.json();
    const scanning = s.is_scanning ? " · scanning…" : "";
    if (Date.now() >= errorUntil) {
      setStatus(s.db_healthy, `${s.total_indexed_files.toLocaleString()} files indexed${scanning}`);
    }
  } catch (err) {
    setStatus(false, `engine unreachable: ${err.name || "Error"}: ${err.message || err} (base="${ENGINE_BASE}")`, true);
  } finally {
    setTimeout(pollStatus, 4000);
  }
}

const ERROR_DISPLAY_MS = 6000;

function setStatus(healthy, text, isError = false) {
  if (isError) {
    errorUntil = Date.now() + ERROR_DISPLAY_MS;
  }
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
