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
  editMenu: document.getElementById("editMenu"),
  editMenuToggle: document.getElementById("editMenuToggle"),
  editMenuList: document.getElementById("editMenuList"),
};

// ---------- Visible Edit menu (Cut/Copy/Paste/Select All) ----------
//
// The native window's own Cmd+C/V/X/A and right-click menu already work
// (see DeepScanWindow.swift's Edit menu), but that's easy to miss on a
// window with no visible menu bar of its own — this gives the same
// actions a button people can actually see and click. The query textarea
// is the only editable field in the app, so these all operate on it
// directly rather than relying on document.execCommand, which loses the
// textarea's selection the instant focus moves to the toggle button.
let savedSelection = { start: 0, end: 0 };
els.queryInput.addEventListener("blur", () => {
  savedSelection = { start: els.queryInput.selectionStart, end: els.queryInput.selectionEnd };
});

els.editMenuToggle.addEventListener("click", (e) => {
  e.stopPropagation();
  const isOpen = !els.editMenuList.hidden;
  els.editMenuList.hidden = isOpen;
  els.editMenuToggle.setAttribute("aria-expanded", String(!isOpen));
});

document.addEventListener("click", (e) => {
  if (!els.editMenu.contains(e.target)) {
    els.editMenuList.hidden = true;
    els.editMenuToggle.setAttribute("aria-expanded", "false");
  }
});

els.editMenuList.addEventListener("click", async (e) => {
  const action = e.target.dataset.editAction;
  if (!action) return;
  els.editMenuList.hidden = true;
  els.editMenuToggle.setAttribute("aria-expanded", "false");
  try {
    await runEditAction(action);
  } catch (err) {
    console.error("[DeepScan] edit action failed:", err);
    setStatus(false, `${action} failed: ${err.name || "Error"}: ${err.message || err}`, true);
  }
});

async function runEditAction(action) {
  const { start, end } = savedSelection;
  const value = els.queryInput.value;

  if (action === "selectAll") {
    els.queryInput.focus();
    els.queryInput.select();
    return;
  }

  if (action === "copy" || action === "cut") {
    const text = start === end ? value : value.slice(start, end);
    await navigator.clipboard.writeText(text);
    if (action === "cut" && start !== end) {
      els.queryInput.value = value.slice(0, start) + value.slice(end);
      els.queryInput.focus();
      els.queryInput.setSelectionRange(start, start);
      els.queryInput.dispatchEvent(new Event("input"));
    }
    return;
  }

  if (action === "paste") {
    const text = await navigator.clipboard.readText();
    els.queryInput.value = value.slice(0, start) + text + value.slice(end);
    els.queryInput.focus();
    const pos = start + text.length;
    els.queryInput.setSelectionRange(pos, pos);
    els.queryInput.dispatchEvent(new Event("input"));
  }
}

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

// A dropped .app (or any other folder) isn't a real single file — the
// browser hands over a File object for it, but reading its bytes either
// throws or silently returns nothing useful. Without a guard here, that
// left the UI stuck on "searching by X…" forever with no error shown,
// since nothing ever caught it.
const MAX_IMAGE_QUERY_BYTES = 25 * 1024 * 1024;

async function handleImageFile(file) {
  els.dropLabel.textContent = `searching by “${file.name}”…`;
  try {
    if (!file.type.startsWith("image/")) {
      throw new Error(`"${file.name}" isn't an image DeepScan can search by`);
    }
    if (file.size > MAX_IMAGE_QUERY_BYTES) {
      throw new Error(`"${file.name}" is too large to search by (${Math.round(file.size / 1024 / 1024)}MB)`);
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    await search({ image_query_bytes: Array.from(bytes), scope: state.scope });
  } catch (err) {
    console.error("[DeepScan] image query failed:", err);
    setStatus(false, `image query failed: ${err.name || "Error"}: ${err.message || err}`, true);
  } finally {
    els.dropLabel.textContent = "drag an image or icon here";
  }
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
    setStatus(false, `search failed: ${err.name || "Error"}: ${err.message || err} (base="${ENGINE_BASE}")`, true);
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
