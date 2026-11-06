const $ = (id) => document.getElementById(id);
const SLIDERS = ["exposure", "contrast", "highlights", "shadows", "whites", "blacks", "temperature", "tint", "saturation"];
const SAVE_SHORTCUT = navigator.platform.startsWith("Mac") ? "⌘S" : "Ctrl+S";
const LOOKS = ["neutral", "flat", "vivid"];
const TOOL_TEXT = {
  crop: "Drag over what the saved DNG should keep. This doesn't change the colors.",
  area: "Drag over the part of the scan that is picture (no rebate, no holder). Color and exposure are measured here. This doesn't change the crop.",
  base: "Drag over clear, unexposed film between or beside the frames, away from holes and markings.",
};

let current = null;   // { file, auto, settings }
let tool = null;      // null | "crop" | "area" | "base" | "show"
let timer = null;
let saveTimer = null;
let opening = null;       // the frame being loaded
let frames = [];          // what the frame list shows
const selected = new Set();
let lastChecked = null;   // for shift-click
let batch = null;         // the running export's progress
let lastPath = null;
let rendering = false;  // one preview at a time; changes meanwhile ask for one more
let renderAgain = false;

// In the Mac app the shell does what a page can't.
const shell = window.webkit?.messageHandlers?.reversal;
async function api(path, body) {
  const res = await fetch(path, body ? { method: "POST", keepalive: true, headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {});
  if (!res.ok) {
    let msg = `${res.status}`;
    try { msg = (await res.json()).error; } catch {}
    throw new Error(msg);
  }
  return res;
}

const adj = () => current.settings.adjust;
const hints = () => current.settings.hints;
// The status bar holds one line; anything longer opens from Details.
function status(text) {
  const [head, ...rest] = text.split("\n");
  $("status").textContent = head;
  $("status").title = head;
  $("status-more").textContent = text;
  $("status-details").hidden = !rest.length;
  showDetails(false);
}
function showDetails(on) {
  $("status-more").hidden = !on;
  $("status-details").setAttribute("aria-expanded", String(on));
}
$("status-details").addEventListener("click", () => showDetails($("status-more").hidden));


// ---------- Files ----------
async function loadFiles() {
  const data = await (await api("/api/files")).json();
  $("folder").textContent = data.folder.split("/").filter(Boolean).pop() ?? data.folder;
  $("choose-folder").title = shell ? `${data.folder}\nOpen another folder…` : data.folder;
  frames = data.files;
  const names = new Set(frames.map((f) => f.name));
  for (const name of selected) if (!names.has(name)) selected.delete(name);
  const list = $("files");
  const scroll = list.scrollTop;
  list.replaceChildren(...frames.map(frameRow));
  list.scrollTop = scroll;
  if (!frames.length) $("message").textContent = "No raw files in this folder. Start Reversal on a folder with NEF, CR2, ARW, RAF or DNG scans.";
  markCurrent(opening ?? current?.file);
  updateSaveButton();
}

function frameRow(f) {
  const li = document.createElement("li");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = selected.has(f.name);
  box.dataset.name = f.name;
  box.setAttribute("aria-label", `Select ${f.name}`);
  box.addEventListener("click", (e) => checkFrame(f.name, box.checked, e.shiftKey));
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.name = f.name;
  const name = document.createElement("span"); name.className = "name"; name.textContent = f.name; name.title = f.name;
  const state = document.createElement("span");
  b.append(name, state);
  b.addEventListener("click", () => open(f.name));
  li.append(box, b);
  showFrameState(f, state);
  return li;
}

function showFrameState(f, state) {
  const saving = batch?.task === "save" && batch.active.includes(f.name);
  state.textContent = saving ? "saving" : f.edited ? "edited" : f.saved ? "saved" : "";
  state.className = `state${f.edited && !saving ? " edited" : ""}${f.manualSetup ? " manual" : ""}`;
  state.title = f.manualSetup ? "Film base or measured area set by hand" : f.edited ? "Changes not saved to a DNG yet" : "";
}

function showFrameStates() {
  for (const b of document.querySelectorAll("#files button")) {
    const f = frames.find((x) => x.name === b.dataset.name);
    if (f) showFrameState(f, b.querySelector(".state"));
  }
}

function checkFrame(name, on, range) {
  const names = frames.map((f) => f.name);
  const from = range && lastChecked ? names.indexOf(lastChecked) : -1;
  const at = names.indexOf(name);
  const span = from < 0 ? [name] : names.slice(Math.min(from, at), Math.max(from, at) + 1);
  for (const n of span) on ? selected.add(n) : selected.delete(n);
  lastChecked = name;
  for (const box of document.querySelectorAll("#files input")) box.checked = selected.has(box.dataset.name);
  updateSaveButton();
}

$("select-all-box").addEventListener("change", (e) => {
  selected.clear();
  if (e.target.checked) for (const f of frames) selected.add(f.name);
  for (const box of document.querySelectorAll("#files input")) box.checked = selected.has(box.dataset.name);
  updateSaveButton();
});

function updateSaveButton() {
  const button = $("export");
  shell?.postMessage({ action: "busy", busy: !!batch });
  $("frame-count").textContent = !frames.length ? "" : selected.size ? `${selected.size} of ${frames.length}` : `${frames.length} frame${frames.length > 1 ? "s" : ""}`;
  $("progress").hidden = batch?.task !== "save";
  const saving = batch?.task === "save";
  button.classList.toggle("cancel", saving);
  button.title = saving ? "Frames being saved now finish; the rest don't start" : SAVE_SHORTCUT;
  if (saving) {
    button.textContent = "Stop saving";
    button.disabled = false;
    return;
  }
  button.textContent = selected.size ? `Save ${selected.size} DNG${selected.size > 1 ? "s" : ""}` : "Save DNG";
  button.disabled = !!batch || (!selected.size && !current);
  const box = $("select-all-box");
  box.checked = frames.length > 0 && selected.size === frames.length;
  box.indeterminate = selected.size > 0 && selected.size < frames.length;
}

function markCurrent(name) {
  for (const b of document.querySelectorAll("#files button")) {
    b.setAttribute("aria-current", String(b.dataset.name === name));
    if (b.dataset.name === name) b.scrollIntoView({ block: "nearest" });
  }
  const at = frames.findIndex((f) => f.name === name);
  $("position-name").textContent = at < 0 ? "" : name;
  $("position-index").textContent = at < 0 ? "" : `${at + 1} of ${frames.length}`;
}

// ---------- Remembering settings ----------
// Each frame keeps its settings on disk from the first change, so frames can be saved together.
async function rememberSettings(leaving = false) {
  clearTimeout(saveTimer);
  // A roll is rewriting the sidecars. A frame that stays open sends its changes when the roll
  // ends; one being left can't wait, so it sends only its adjustments.
  const rolling = batch?.task === "roll";
  if (!current || (rolling && !leaving)) return;
  const frame = current;
  const { file, settings } = frame;
  const json = JSON.stringify(settings);
  if (json === frame.remembered) return;
  const state = await (await api("/api/settings", { file, settings, adjust_only: rolling })).json();
  frame.remembered = json;
  const f = frames.find((x) => x.name === file);
  if (f) { Object.assign(f, state); showFrameStates(); }
}

function rememberSoon() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => rememberSettings().catch((e) => status(`Could not keep your settings: ${e.message}`)), 300);
}

async function open(name) {
  try {
    await rememberSettings(true);
  } catch (e) {
    status(`Could not keep your settings, so this frame stays open: ${e.message}`);
    return;
  }
  clearTimeout(timer);
  clearTimeout(areaTimer);
  opening = name;
  current = null;  // no edits while nothing is open
  markCurrent(name);
  setTool(null);
  $("message").textContent = "Decoding the raw file…";
  $("message").hidden = false;
  $("view").hidden = true;
  setBusy(true);
  try {
    const data = await (await api(`/api/load?file=${encodeURIComponent(name)}`)).json();
    if (opening !== name) return;  // another frame was chosen meanwhile
    current = { file: name, auto: data.auto, settings: data.settings, defaultAreaPercent: data.defaultAreaPercent, warning: data.warning, baseFound: data.baseFound };
    current.settings.hints ??= {};
    current.remembered = JSON.stringify(current.settings);
    syncControls();
    await refresh();
    status("");
  } catch (e) {
    if (opening === name) $("message").textContent = `Could not open ${name}: ${e.message}`;
  } finally {
    if (opening === name) {
      opening = null;
      setBusy(false);
    }
  }
}

function setBusy(busy) {
  $("reset").disabled = busy || !current;
  const empty = !current && !opening;
  for (const part of ["frame-tools", "develop", "setup"]) $(part).inert = empty;
  updateSaveButton();
  if (busy && !batch) $("export").disabled = true;
}

// ---------- Controls ----------
function fmt(id, v) {
  if (id === "straighten") return `${v > 0 ? "+" : ""}${v.toFixed(1)}°`;
  if (id === "area-percent") return `${Math.round(v)}%`;
  return `${v > 0 ? "+" : ""}${Math.round(v)}`;
}

function showValue(id, v) {
  const range = $(id), box = $(`${id}-val`);
  range.value = v;
  paintSlider(range);
  if (document.activeElement !== box) box.value = fmt(id, Number(v));
}

// The filled part of a track runs from the center, or from the low end for a size.
function paintSlider(range) {
  const min = Number(range.min), max = Number(range.max);
  const at = (Number(range.value) - min) / (max - min);
  const origin = range.classList.contains("from-min") ? 0 : 0.5;
  range.style.setProperty("--from", Math.min(at, origin));
  range.style.setProperty("--to", Math.max(at, origin));
}

// What could spoil this frame's conversion stays in view with the controls that fix it.
function showSetupNote() {
  const guessing = !current.baseFound && !hints().base && !hints().base_rgb;
  const note = current.warning ?? (guessing ? "No clear film found on this frame, so its colors may be off. Pick the film base, or match it across the roll." : "");
  $("setup-note").textContent = note;
  $("setup-note").hidden = !note;
}

function syncControls() {
  const a = adj();
  for (const id of [...SLIDERS, "straighten"]) showValue(id, a[id]);
  for (const look of LOOKS) $(`look-${look}`).setAttribute("aria-pressed", String(a.look === look));
  const percent = hints().area_percent ?? current.defaultAreaPercent;
  showValue("area-percent", percent);
  $("flip").setAttribute("aria-pressed", String(a.flip_h));
  const cropMode = !a.crop ? "full" : sameRect(a.crop, current.settings.estimate.area) ? "frame" : "draw";
  for (const m of ["full", "frame", "draw"]) $(`crop-${m}`).setAttribute("aria-pressed", String(cropMode === m || tool === "crop" && m === "draw"));
  $("area-auto").setAttribute("aria-pressed", String(!hints().analysis));
  $("area-draw").setAttribute("aria-pressed", String(!!hints().analysis || tool === "area"));
  $("base-auto").setAttribute("aria-pressed", String(!hints().base && !hints().base_rgb));
  $("roll-state").hidden = !!hints().base || !hints().base_rgb;
  showSetupNote();
  $("base-draw").setAttribute("aria-pressed", String(!!hints().base || tool === "base"));
  $("show-areas").setAttribute("aria-pressed", String(tool === "show"));
}

const sameRect = (a, b) => b && ["x0", "y0", "x1", "y1"].every((k) => Math.abs(a[k] - b[k]) < 1e-4);

function changed(delay = 0) {
  if (!current) return;
  syncControls();
  rememberSoon();
  clearTimeout(timer);
  timer = setTimeout(refresh, delay);
}

for (const id of [...SLIDERS, "straighten"]) {
  $(id).title = "Double-click to reset";
  $(id).addEventListener("input", (e) => { if (current) { adj()[id] = Number(e.target.value); changed(); } });
  $(id).addEventListener("dblclick", () => { if (current) { adj()[id] = 0; changed(0); } });
}

for (const look of LOOKS) {
  $(`look-${look}`).addEventListener("click", () => { if (current) { adj().look = look; changed(0); } });
}

let areaTimer = null;
function setAreaPercent(percent) {
  if (!current) return;
  // The default is no choice at all, however it was reached.
  hints().area_percent = percent === current.defaultAreaPercent ? null : percent;
  syncControls();
  clearTimeout(areaTimer);
  areaTimer = setTimeout(() => setHint("analysis", null), 150);
}
$("area-percent").addEventListener("input", (e) => setAreaPercent(Number(e.target.value)));
$("area-percent").addEventListener("dblclick", () => setAreaPercent(null));

// A value box drives its slider, so a typed number goes the same way as a drag.
for (const id of [...SLIDERS, "straighten", "area-percent"]) {
  const range = $(id), box = $(`${id}-val`);
  const set = (v) => {
    const clamped = Math.min(Number(range.max), Math.max(Number(range.min), v));
    if (clamped === Number(range.value)) return;
    range.value = clamped;
    range.dispatchEvent(new Event("input"));
  };
  const typed = () => parseFloat(box.value.replace("−", "-").replace(/[^\d.+-]/g, ""));
  const restore = () => { box.value = fmt(id, Number(range.value)); };
  // The first click takes the whole value: without this, letting go of the mouse drops the
  // selection in WebKit and typing lands beside the old number.
  box.addEventListener("mousedown", (e) => {
    if (document.activeElement !== box) { e.preventDefault(); box.focus(); }
  });
  box.addEventListener("focus", () => box.select());
  box.addEventListener("blur", () => {
    const v = typed();
    if (!Number.isNaN(v)) set(v);
    restore();
  });
  box.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter") box.blur();
    else if (e.key === "Escape") { restore(); box.blur(); }
    else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const step = Number(range.step) * (e.shiftKey ? 10 : 1);
      set(Number(range.value) + (e.key === "ArrowUp" ? step : -step));
      restore();
      box.select();
    }
  });
}
document.querySelectorAll("input[type=range]").forEach(paintSlider);

// Mirror the image as it is shown now: with an odd quarter turn the stored
// "mirror, then rotate" order needs the opposite rotation.
$("flip").addEventListener("click", () => {
  if (!current) return;
  const a = adj();
  a.flip_h = !a.flip_h;
  a.rotate = (4 - a.rotate) % 4;
  changed(0);
});
$("rotate").addEventListener("click", () => { if (current) { adj().rotate = (adj().rotate + 1) % 4; changed(0); } });

$("crop-full").addEventListener("click", () => { if (current) { adj().crop = null; setTool(null); changed(0); } });
$("crop-frame").addEventListener("click", () => { if (current) { adj().crop = { ...current.settings.estimate.area }; setTool(null); changed(0); } });
$("crop-draw").addEventListener("click", () => setTool(tool === "crop" ? null : "crop"));
$("area-draw").addEventListener("click", () => setTool(tool === "area" ? null : "area"));
$("base-draw").addEventListener("click", () => setTool(tool === "base" ? null : "base"));
$("show-areas").addEventListener("click", () => setTool(tool === "show" ? null : "show"));
$("area-auto").addEventListener("click", () => setHint("analysis", null));
$("base-auto").addEventListener("click", () => setHint("base", null));
$("base-roll").addEventListener("click", openRollFlow);
$("tool-done").addEventListener("click", () => setTool(null));
document.addEventListener("keydown", (e) => {
  if ($("roll-flow").open || $("about").open) return;
  if (e.key === "Escape") {
    if (tool) setTool(null);
    showDetails(false);
    return;
  }
  // ⌘S or Ctrl+S saves, and never stops a save already running.
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    if (batch?.task !== "save" && !$("export").disabled) $("export").click();
    return;
  }
  // Up and down walk the frames, unless a slider or text box has the keys.
  if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.target.matches("input[type=range], input[type=text]")) {
    e.preventDefault();
    const at = frames.findIndex((f) => f.name === (opening ?? current?.file));
    const next = frames[e.key === "ArrowUp" ? Math.max(at - 1, 0) : Math.min(at + 1, frames.length - 1)];
    if (next && next.name !== (opening ?? current?.file)) open(next.name);
  }
});
// In the app, a right-click shouldn't offer a web page's Reload.
if (shell) document.addEventListener("contextmenu", (e) => { if (!e.target.matches("input[type=text], #status, #status-more") && !e.target.closest("#about")) e.preventDefault(); });

$("reset").addEventListener("click", () => {
  if (!current) return;
  for (const id of SLIDERS) adj()[id] = 0;
  adj().look = "neutral";
  changed(0);
  status("Tone and color are back to the automatic conversion.");
});

$("export").addEventListener("click", async () => {
  if (batch) {
    if (batch.task === "save") api("/api/export/cancel", {}).catch(() => {});
    return;
  }
  const files = selected.size ? frames.map((f) => f.name).filter((n) => selected.has(n)) : current ? [current.file] : [];
  if (!files.length) return;
  $("reveal").hidden = true;
  try {
    await rememberSettings();
    batch = await (await api("/api/export", { files })).json();
    updateSaveButton();
    followBatch();
  } catch (e) {
    status(`Could not save: ${e.message}`);
  }
});

// ---------- Matching the film base across a roll ----------
// Three steps, because this reaches past the frame on screen: say what will happen, read every
// frame without changing anything, then let the user choose which readings count.
let roll = null;  // { step, files, readings, ticked, voted }
const SOURCE = { picked: "Your pick", detected: "Detected", unsure: "No clear film" };

function rollFrames() {
  const names = frames.map((f) => f.name);
  return selected.size ? names.filter((n) => selected.has(n)) : names;
}

function openRollFlow() {
  if (batch) { status("Wait for the current work to finish."); return; }
  roll = { step: "intro", files: rollFrames() };
  showRollFlow();
  $("roll-flow").showModal();
}

function closeRollFlow() {
  roll = null;
  if ($("roll-flow").open) $("roll-flow").close();
}

function rollButtons(cancel, go, enabled = true) {
  $("roll-cancel").textContent = cancel;
  $("roll-go").hidden = !go;
  $("roll-go").textContent = go ?? "";
  $("roll-go").disabled = !enabled;
}

function showRollFlow() {
  const body = $("roll-body");
  const count = roll.files?.length ?? 0;
  if (roll.step === "intro") {
    const scope = selected.size ? `the ${count} ticked frame${count > 1 ? "s" : ""}` : `all ${count} frames in this folder`;
    body.innerHTML = `<p>Frames from one roll have the same clear film, so they can share one film base. This helps when some scans don't show the film's edge.</p>
      <p>Reversal reads the film base on <b></b>: your pick where a frame has one, otherwise the clear film it detects. Nothing changes yet. You then see each reading and choose which ones count.</p>
      <p>Color curves and exposure stay each frame's own.</p>`;
    body.querySelector("b").textContent = scope;
    if (count < 2) body.insertAdjacentHTML("beforeend", `<p class="warn">Tick at least two frames, or none to use the whole folder.</p>`);
    rollButtons("Cancel", `Read ${count} frame${count === 1 ? "" : "s"}`, count >= 2);
  } else if (roll.step === "reading" || roll.step === "applying") {
    body.innerHTML = `<p id="roll-progress"></p><div class="progress"><div></div></div>`;
    showRollProgress();
    rollButtons("Stop", null);
  } else {
    showRollReview();
  }
}

function showRollProgress() {
  const progress = $("roll-progress");
  if (!progress || !batch) return;
  const verb = roll.step === "reading" ? "Reading the film base on frame" : "Preparing frame";
  progress.textContent = `${verb} ${Math.min(batch.done + batch.failed + 1, batch.total)} of ${batch.total}…`;
  progress.nextElementSibling.firstElementChild.style.width = `${100 * (batch.done + batch.failed) / batch.total}%`;
}

function showRollReview() {
  const body = $("roll-body");
  const scroll = body.querySelector("#roll-table")?.scrollTop ?? 0;
  const applied = roll.readings.filter((r) => r.source !== "picked");
  const lost = roll.issues.length ? `<p class="warn" id="roll-issues"></p>` : "";
  const noClear = roll.readings.every((r) => r.source === "unsure");
  body.innerHTML = `<p>${noClear
      ? `<span class="warn">No frame shows clear film.</span> Cancel, pick the film base on a frame that does, and start again.`
      : "Ticked frames decide the roll's film base. Untick any that look wrong; a good one or two is enough."}</p>
    ${lost}<div id="roll-table"></div><div id="roll-result"></div>`;
  if (lost) body.querySelector("#roll-issues").textContent = roll.issues.map((i) => `${i.file}: ${i.message}`).join("\n");
  const table = body.querySelector("#roll-table");
  roll.readings.forEach((r, i) => {
    const row = document.createElement("label");
    row.className = "roll-row";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = roll.ticked.has(r.file);
    box.addEventListener("change", () => {
      box.checked ? roll.ticked.add(r.file) : roll.ticked.delete(r.file);
      voteRoll();
    });
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = r.file;
    const note = document.createElement("span");
    const differs = roll.voted?.differs[r.file];
    note.className = `note${r.source === "unsure" || differs ? " warn" : ""}`;
    note.textContent = differs ? `${SOURCE[r.source]}, differs` : SOURCE[r.source];
    if (differs) note.title = "This reading is far from the roll's. It may be misread, or from different film.";
    row.append(box, name, note);
    table.append(row);
  });
  table.scrollTop = scroll;
  const result = body.querySelector("#roll-result");
  const voters = roll.ticked.size;
  if (roll.voted?.base_rgb && voters) {
    const keeps = roll.readings.length - applied.length;
    result.textContent = `Film base from ${voters} frame${voters > 1 ? "s" : ""}.${keeps ? ` ${keeps} with a pick keep${keeps > 1 ? "" : "s"} it.` : ""}`;
  } else {
    result.textContent = "Tick at least one frame.";
  }
  rollButtons("Cancel", `Use on ${applied.length} frame${applied.length === 1 ? "" : "s"}`, voters > 0 && applied.length > 0);
}

// The median and what strays from it come from the engine, like every other measurement.
async function voteRoll() {
  const review = roll;
  const voters = review.readings.filter((r) => review.ticked.has(r.file));
  if (!voters.length) { review.voted = null; showRollReview(); return; }
  try {
    const { base_rgb, differs } = await (await api("/api/roll/vote", { bases: voters.map((r) => r.base_rgb), readings: review.readings.map((r) => r.base_rgb) })).json();
    if (roll !== review) return;
    review.voted = { base_rgb, differs: Object.fromEntries(review.readings.map((r, i) => [r.file, differs[i]])) };
    showRollReview();
  } catch (e) {
    closeRollFlow();
    status(`Could not measure the roll: ${e.message}`);
  }
}

async function startRollTask(step, url, body) {
  roll.step = step;
  showRollFlow();
  try {
    await rememberSettings();
    batch = await (await api(url, body)).json();
    updateSaveButton();
    followBatch();
  } catch (e) {
    closeRollFlow();
    status(`Could not measure the roll: ${e.message}`);
  }
}

$("roll-go").addEventListener("click", () => {
  if (roll?.step === "intro") {
    $("reveal").hidden = true;
    startRollTask("reading", "/api/roll", { files: roll.files });
  } else if (roll?.step === "review") {
    const bases = roll.readings.filter((r) => roll.ticked.has(r.file)).map((r) => r.base_rgb);
    startRollTask("applying", "/api/roll/apply", { files: roll.readings.map((r) => r.file), bases });
  }
});
function cancelRollFlow() {
  if (roll?.step === "reading" || roll?.step === "applying") {
    $("roll-cancel").disabled = true;
    $("roll-cancel").textContent = "Stopping…";
    api("/api/export/cancel", {}).catch(() => {});
  } else {
    closeRollFlow();
  }
}
$("roll-cancel").addEventListener("click", cancelRollFlow);
// Escape means the same as the button beside the action.
$("roll-flow").addEventListener("cancel", (e) => { e.preventDefault(); cancelRollFlow(); });

async function rollReadingsDone(finished) {
  $("roll-cancel").disabled = false;
  if (finished.canceled) {
    closeRollFlow();
    status("Stopped. No frame changed.");
    return;
  }
  const readings = await (await api("/api/roll/readings")).json();
  // Frames in list order, whichever finished first; a frame with no clear film gets no say.
  const order = frames.map((f) => f.name);
  readings.sort((a, b) => order.indexOf(a.file) - order.indexOf(b.file));
  roll ??= {};
  Object.assign(roll, { step: "review", readings, issues: finished.issues, ticked: new Set(readings.filter((r) => r.source !== "unsure").map((r) => r.file)), voted: null });
  showRollFlow();
  if (!$("roll-flow").open) $("roll-flow").showModal();
  voteRoll();
}

async function followBatch() {
  try {
    batch = await (await api("/api/export/status")).json();
  } catch (e) {
    batch = null;
    updateSaveButton();
    status(`Lost track of saving: ${e.message}`);
    return;
  }
  const inRoll = batch.task === "roll" || batch.task === "roll-read";
  // Reloaded during a roll: the flow picks up where it was.
  if (inRoll && !roll) {
    roll = { step: batch.task === "roll" ? "applying" : "reading" };
    showRollFlow();
    if (!$("roll-flow").open) $("roll-flow").showModal();
  }
  if (batch.running) {
    if (inRoll) {
      showRollProgress();
    } else {
      status(`Saving ${Math.min(batch.done + batch.failed + 1, batch.total)} of ${batch.total}…`);
      $("progress").firstElementChild.style.width = `${100 * (batch.done + batch.failed) / batch.total}%`;
    }
    showFrameStates();
    setTimeout(followBatch, 150);
    return;
  }
  const finished = batch;
  batch = null;
  if (finished.task === "roll-read") {
    updateSaveButton();
    rollReadingsDone(finished).catch((e) => {
      closeRollFlow();
      status(`Could not measure the roll: ${e.message}`);
    });
    return;
  }
  if (inRoll) { $("roll-cancel").disabled = false; closeRollFlow(); }
  await loadFiles();
  if (finished.task === "roll") {
    await reopenAfterRoll();
    status(rollSummary(finished));
    return;
  }
  if (finished.done === finished.total) selected.clear();
  updateSaveButton();
  for (const box of document.querySelectorAll("#files input")) box.checked = selected.has(box.dataset.name);
  status(batchSummary(finished));
  lastPath = finished.last_path;
  $("reveal").hidden = !(shell && lastPath);
}

// The open frame's base and measurements changed on disk: read them back, and keep the slider
// moves made while measuring. A frame being opened already reads them.
async function reopenAfterRoll() {
  if (!current) return;
  const { file, settings: { adjust } } = current;
  current.remembered = JSON.stringify(current.settings);
  await open(file);
  if (current?.file === file) {
    current.settings.adjust = adjust;
    changed(0);
  }
}

function rollSummary({ done, total, issues, canceled }) {
  const head = canceled ? "Stopped. No frame changed."
    : issues.length ? "The roll's film base didn't reach every frame."
    : `${total} frames now share one film base. Detect on a frame goes back to its own.`;
  return [head, ...issues.map((i) => (i.file ? `${i.file}: ${i.message}` : i.message))].join("\n");
}

function batchSummary({ done, total, issues, canceled, last_path }) {
  const where = last_path ? ` to ${last_path.slice(0, last_path.lastIndexOf("/"))}` : "";
  const head = total === 1 && done === 1 ? `Saved ${last_path}` : `Saved ${done === total ? done : `${done} of ${total}`} DNGs${where}${canceled ? ", then stopped" : ""}.`;
  return [head, ...issues.map((i) => `${i.file}: ${i.message}`)].join("\n");
}

$("reveal").addEventListener("click", () => { if (shell && lastPath) shell.postMessage({ action: "reveal", path: lastPath }); });

async function setHint(key, rect) {
  if (!current) return;
  if (batch?.task === "roll") { status("Measuring the roll; try again when it's done."); return; }
  const file = current.file;
  const wasAreaCrop = adj().crop && sameRect(adj().crop, current.settings.estimate.area);
  hints()[key] = rect;
  if (key === "base") hints().base_rgb = null;
  status("Measuring…");
  try {
    const estimate = await (await api("/api/estimate", { file, hints: hints() })).json();
    if (current?.file !== file) return;
    current.settings.estimate = estimate;
    current.warning = null;
    if (wasAreaCrop) adj().crop = { ...current.settings.estimate.area };
    status(key === "base" ? (rect ? "Film base set from your selection." : "Film base now read from this frame.")
                          : (rect ? "Image area set from your selection." : "Measuring the center of the scan."));
  } catch (e) {
    if (current?.file === file) status(`Could not measure: ${e.message}`);
  }
  if (current?.file === file) changed(0);
}

// ---------- Tools and overlay ----------
// Region tools show the whole scan (straightened, mirrored, rotated) with the areas drawn on it.
function setTool(t) {
  tool = t;
  $("overlay").toggleAttribute("hidden", !t); // SVG elements don't reflect .hidden
  $("toolbar").hidden = !t;
  $("tool-text").textContent = t === "show" ? "Showing the crop, measured area and film base." : TOOL_TEXT[t] ?? "";
  if (current) changed(0);
}

// Scan (normalized, straightened) <-> displayed full view: mirror first, then quarter turns.
function scanToView([x, y], a) {
  if (a.flip_h) x = 1 - x;
  switch (a.rotate % 4) {
    case 1: return [1 - y, x];
    case 2: return [1 - x, 1 - y];
    case 3: return [y, 1 - x];
    default: return [x, y];
  }
}
function viewToScan([u, v], a) {
  let x, y;
  switch (a.rotate % 4) {
    case 1: [x, y] = [v, 1 - u]; break;
    case 2: [x, y] = [1 - u, 1 - v]; break;
    case 3: [x, y] = [1 - v, u]; break;
    default: [x, y] = [u, v];
  }
  return [a.flip_h ? 1 - x : x, y];
}
function rectFromPoints(p, q) {
  return { x0: Math.min(p[0], q[0]), y0: Math.min(p[1], q[1]), x1: Math.max(p[0], q[0]), y1: Math.max(p[1], q[1]) };
}
function svgRect(r, cls) {
  const a = adj();
  const v = rectFromPoints(scanToView([r.x0, r.y0], a), scanToView([r.x1, r.y1], a));
  return `<rect class="${cls}" x="${v.x0}" y="${v.y0}" width="${v.x1 - v.x0}" height="${v.y1 - v.y0}"/>`;
}
function drawOverlay(drag) {
  if (!current || !tool) return;
  const e = current.settings.estimate;
  let s = svgRect(e.area, "area") + svgRect(e.base_area, "base");
  if (adj().crop) s += svgRect(adj().crop, "crop");
  if (drag) s += `<rect class="drag" x="${drag.x0}" y="${drag.y0}" width="${drag.x1 - drag.x0}" height="${drag.y1 - drag.y0}"/>`;
  $("overlay").innerHTML = s;
}

let dragStart = null;
const eventPoint = (ev) => {
  const b = $("overlay").getBoundingClientRect();
  return [Math.min(1, Math.max(0, (ev.clientX - b.left) / b.width)), Math.min(1, Math.max(0, (ev.clientY - b.top) / b.height))];
};
$("overlay").addEventListener("pointerdown", (ev) => {
  if (!["crop", "area", "base"].includes(tool)) return;
  dragStart = eventPoint(ev);
  $("overlay").setPointerCapture(ev.pointerId);
});
$("overlay").addEventListener("pointermove", (ev) => {
  if (dragStart) drawOverlay(rectFromPoints(dragStart, eventPoint(ev)));
});
$("overlay").addEventListener("pointerup", (ev) => {
  if (!dragStart) return;
  const view = rectFromPoints(dragStart, eventPoint(ev));
  dragStart = null;
  if (view.x1 - view.x0 < 0.005 || view.y1 - view.y0 < 0.005) { drawOverlay(); return; }
  const a = adj();
  const scan = rectFromPoints(viewToScan([view.x0, view.y0], a), viewToScan([view.x1, view.y1], a));
  const t = tool;
  if (t === "crop") { a.crop = scan; setTool(null); }
  else { setTool(null); setHint(t === "area" ? "analysis" : "base", scan); }
});

// ---------- Preview, fit and histogram ----------
async function refresh() {
  if (!current) return;
  if (rendering) { renderAgain = true; return; }
  rendering = true;
  const file = current.file;
  try {
    const res = await api("/api/preview", { file, settings: current.settings, full: !!tool });
    const clipped = Number(res.headers.get("X-Clipped") || 0);
    const blob = await res.blob();
    if (file !== current?.file) return;
    const img = $("photo");
    const old = img.src;
    // Not img.decode(): it can stay pending for as long as the page is hidden.
    await new Promise((done) => { img.onload = img.onerror = done; img.src = URL.createObjectURL(blob); });
    if (old.startsWith("blob:")) URL.revokeObjectURL(old);
    if (file !== current?.file) return;
    $("view").hidden = false;
    $("message").hidden = true;
    fit();
    drawOverlay();
    histogram(img, clipped);
  } catch (e) {
    if (file === current?.file) status(`Preview failed: ${e.message}`);
  } finally {
    rendering = false;
    if (renderAgain) { renderAgain = false; refresh(); }
  }
}

function fit() {
  const img = $("photo");
  if (!img.naturalWidth) return;
  const stage = $("stage");
  const pad = 56;
  const scale = Math.min((stage.clientWidth - pad) / img.naturalWidth, (stage.clientHeight - pad) / img.naturalHeight);
  $("view").style.width = `${Math.max(1, img.naturalWidth * scale)}px`;
  $("view").style.height = `${Math.max(1, img.naturalHeight * scale)}px`;
}
new ResizeObserver(fit).observe($("stage"));

const histCanvas = document.createElement("canvas");
function histogram(img, storedClipped) {
  const w = Math.min(400, img.naturalWidth), h = Math.round(img.naturalHeight * w / img.naturalWidth);
  histCanvas.width = w; histCanvas.height = h;
  const ctx = histCanvas.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(img, 0, 0, w, h);
  const px = ctx.getImageData(0, 0, w, h).data;
  const bins = [new Uint32Array(256), new Uint32Array(256), new Uint32Array(256)];
  let low = 0, high = 0;
  for (let i = 0; i < px.length; i += 4) {
    const r = px[i], g = px[i + 1], b = px[i + 2];
    bins[0][r]++; bins[1][g]++; bins[2][b]++;
    if (r <= 1 && g <= 1 && b <= 1) low++;
    if (r >= 254 || g >= 254 || b >= 254) high++;
  }
  const n = px.length / 4;
  const c = $("histogram"), g = c.getContext("2d");
  g.clearRect(0, 0, c.width, c.height);
  // Scale to the 99.5th percentile bin so a spike at 0 or 255 doesn't flatten the rest.
  const all = bins.flatMap((b) => Array.from(b.slice(1, 255))).sort((a, b) => a - b);
  const peak = Math.max(1, all[Math.floor(all.length * 0.995)]);
  g.globalCompositeOperation = "lighter";
  const colors = ["rgba(230,70,70,.85)", "rgba(70,200,90,.85)", "rgba(80,130,240,.85)"];
  bins.forEach((bin, k) => {
    g.fillStyle = colors[k];
    g.beginPath();
    g.moveTo(0, c.height);
    for (let x = 0; x < 256; x++) g.lineTo(x * c.width / 255, c.height - Math.min(1, bin[x] / peak) * c.height);
    g.lineTo(c.width, c.height);
    g.fill();
  });
  g.globalCompositeOperation = "source-over";
  const pct = (v) => `${(100 * v / n).toFixed(v / n < 0.01 ? 2 : 1)}%`;
  $("clip-low").textContent = `Shadows ${pct(low)}`;
  $("clip-high").textContent = `Highlights ${pct(high)}`;
  $("clip-low").classList.toggle("over", low / n > 0.005);
  $("clip-high").classList.toggle("over", high / n > 0.005);
  $("clip-high").title = high / n > 0.005 && storedClipped < 0.0005 ? "The DNG keeps these. Lower Whites or use Flat to see them." : "";
  $("stored").hidden = storedClipped < 0.0005;
  $("stored").textContent = `${(storedClipped * 100).toFixed(2)}% of pixels are brighter than the DNG can store. Lower Exposure to keep them.`;
  $("stored").title = $("stored").textContent;
}

api("/api/version").then((r) => r.json()).then(({ version, commit }) => {
  const label = commit ? `${version} · ${commit}` : version;
  $("about-open").textContent = label;
  $("about-title").textContent = `Reversal ${label}`;
}).catch(() => {});
$("about-open").addEventListener("click", () => $("about").showModal());
$("about-close").addEventListener("click", () => $("about").close());
$("about-details").addEventListener("toggle", () => {
  if ($("about-license").textContent) return;
  api("/api/license").then((r) => r.text()).then((text) => { $("about-license").textContent = text; }).catch(() => {});
});

$("choose-folder").disabled = !shell;
updateSaveButton();
setBusy(false);
$("choose-folder").addEventListener("click", () => shell.postMessage({ action: "chooseFolder" }));

loadFiles().catch((e) => { $("message").textContent = `Could not list files: ${e.message}`; });
// Reloaded while saving: pick the progress back up.
api("/api/export/status").then((r) => r.json()).then((s) => { if (s.running) { batch = s; updateSaveButton(); followBatch(); } }).catch(() => {});
addEventListener("pagehide", () => { rememberSettings(true).catch(() => {}); });
