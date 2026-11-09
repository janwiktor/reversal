// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

const $ = (id) => document.getElementById(id);
const SLIDERS = ["exposure", "contrast", "highlights", "shadows", "whites", "blacks", "temperature", "tint", "saturation"];
const SAVE_SHORTCUT = navigator.platform.startsWith("Mac") ? "⌘S" : "Ctrl+S";
const LOOKS = ["neutral", "flat", "vivid"];
const TOOL_TEXT = {
  crop: "Adjust area for the final DNG export.",
  area: "Drag over the exposed image only, avoiding film base, film holder or sprocket holes. This area drives negative conversion, and doesn't affect final crop.",
  base: "Drag over clear unexposed part of the film, avoiding sprocket holes, numbers and film stock markings.",
  show: "Showing the film base, measured area, final crop, and auto-detected film holder area excluded from analysis.",
};

let current = null;   // { file, auto, settings }
let tool = null;      // null | "crop" | "area" | "base" | "show"
let cropDraft = null;  // the open crop frame in scan coordinates; applied by Done or Enter
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
// Everything the app has to say floats at the top of the image, under an open tool's bar, one
// message at a time. A message that needs reading or doing stays until dismissed or replaced; a
// confirmation fades. Anything longer than a line opens from Details.
let noteTimer = 0, noteStays = false;
function status(text = "", { stay = false, reveal = false } = {}) {
  const note = $("note"), [head, ...rest] = text.split("\n");
  clearTimeout(noteTimer);
  noteStays = !!text && (stay || rest.length > 0);
  // A message that stays needs reading, so it interrupts; a confirmation waits its turn.
  note.setAttribute("role", noteStays ? "alert" : "status");
  $("note-text").textContent = head;
  $("note-more").textContent = rest.join("\n");
  showDetails(false);
  $("note-details").hidden = !rest.length;
  $("reveal").hidden = !reveal;
  note.hidden = !text;
  if (text && !noteStays) fadeLater();
}
function fadeLater() {
  noteTimer = setTimeout(() => {
    if ($("note").matches(":hover, :focus-within")) fadeLater();
    else status();
  }, 5000);
}
function showDetails(on) {
  $("note-more").hidden = !on;
  $("note-details").setAttribute("aria-expanded", String(on));
  // Reading the details keeps the message.
  if (on) { clearTimeout(noteTimer); noteStays = true; }
}
$("note-details").addEventListener("click", () => showDetails($("note-more").hidden));
$("note-dismiss").addEventListener("click", () => status());


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
  if (!frames.length) $("message").textContent = "No raw files in this folder. Open a folder with NEF, CR2, ARW, RAF or DNG scans.";
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
  // The label is the checkbox's target, larger than the box on a touch screen. A click on it
  // reaches the box as a second click, so Shift is read from the first.
  const pick = document.createElement("label");
  pick.className = "pick";
  pick.append(box);
  let shiftOnLabel = false;
  pick.addEventListener("click", (e) => { if (e.target === pick) shiftOnLabel = e.shiftKey; });
  box.addEventListener("click", (e) => {
    checkFrame(f.name, box.checked, e.shiftKey || shiftOnLabel);
    shiftOnLabel = false;
  });
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.name = f.name;
  const name = document.createElement("span"); name.className = "name"; name.textContent = f.name; name.title = f.name;
  const state = document.createElement("span");
  b.append(name, state);
  b.addEventListener("click", () => open(f.name));
  li.append(pick, b);
  showFrameState(f, state);
  return li;
}

function showFrameState(f, state) {
  const saving = batch?.task === "save" && batch.active.includes(f.name);
  state.textContent = saving ? "saving" : f.edited ? "edited" : f.saved ? "saved" : "";
  state.className = `state${f.edited && !saving ? " edited" : ""}${f.manualSetup ? " manual" : ""}`;
  state.title = f.manualSetup ? "Custom film base or measured area" : f.edited ? "Changes not saved to a DNG yet" : "";
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

// The open frame's place in the roll, or how many are selected.
let shownName = null;
function showCount() {
  const at = frames.findIndex((f) => f.name === shownName), n = frames.length;
  $("frame-count").textContent = !n ? "" : selected.size ? `${selected.size} of ${n} selected` : at >= 0 ? `${at + 1} of ${n}` : `${n} frame${n > 1 ? "s" : ""}`;
}

// While saving, the button stops the save and fills as frames finish.
function updateSaveButton() {
  const button = $("export");
  shell?.postMessage({ action: "busy", busy: !!batch });
  showCount();
  const saving = batch?.task === "save";
  button.classList.toggle("cancel", saving);
  button.title = saving ? "Finish the frames being saved, then stop." : SAVE_SHORTCUT;
  if (saving) {
    const { done = 0, failed = 0, total = 1 } = batch;
    button.textContent = total > 1 ? `Stop · ${Math.min(done + failed + 1, total)} of ${total}` : "Stop saving";
    button.style.setProperty("--done", `${100 * (done + failed) / total}%`);
    // The button isn't announced as it changes, so progress is told separately.
    const told = `Saving ${Math.min(done + failed + 1, total)} of ${total}`;
    if ($("save-progress").textContent !== told) $("save-progress").textContent = told;
    button.disabled = false;
    return;
  }
  $("save-progress").textContent = "";
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
  shownName = frames.some((f) => f.name === name) ? name : null;
  $("file-name").textContent = shownName ?? "Frames";
  $("file-name").title = shownName ?? "";
  showCount();
}

// ---------- Layout ----------
// The frames list opens and closes in every layout: a column beside the image in landscape, and
// in place of the tools under it in portrait, so the frame being chosen shows above the list.
// On a phone, the bar at the bottom picks which tools show, and closes the list. The column stays
// as it was left; turning the device to portrait with a frame open shows the tools instead.
const framesInPlaceOfTools = matchMedia("(max-width: 599px), (orientation: portrait)");
let framesColumn = true;
function showFrames(on) {
  document.body.dataset.frames = on ? "open" : "closed";
  $("frames-toggle").setAttribute("aria-expanded", String(on));
  if (!framesInPlaceOfTools.matches) framesColumn = on;
}
$("frames-toggle").addEventListener("click", () => showFrames(document.body.dataset.frames !== "open"));
framesInPlaceOfTools.addEventListener("change", () => showFrames(framesInPlaceOfTools.matches ? !shownName : framesColumn));
$("crop-open").addEventListener("click", () => {
  $("crop-open").setAttribute("aria-expanded", String($("crop-open").getAttribute("aria-expanded") !== "true"));
});
for (const button of document.querySelectorAll("#modes button")) {
  button.addEventListener("click", () => {
    document.body.dataset.mode = button.dataset.mode;
    for (const other of document.querySelectorAll("#modes button")) other.setAttribute("aria-pressed", String(other === button));
    showFrames(false);
  });
}

// In portrait the histogram floats over the image, and tapping it folds it to an icon. In
// landscape it has its own place and this button isn't shown.
$("hist-fold").addEventListener("click", () => {
  const folded = !$("hist").hasAttribute("data-folded");
  $("hist").toggleAttribute("data-folded", folded);
  $("hist-fold").setAttribute("aria-expanded", String(!folded));
});

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
  saveTimer = setTimeout(() => rememberSettings().catch((e) => status(`Could not save settings: ${e.message}`, { stay: true })), 300);
}

async function open(name) {
  applyOpenCrop();
  try {
    await measureSettled();
    await rememberSettings(true);
  } catch (e) {
    status(`Could not save settings, so this frame stays open: ${e.message}`, { stay: true });
    return;
  }
  clearTimeout(timer);
  clearTimeout(areaTimer);
  clearTimeout(straightenTimer);
  straightenUnmeasured = false;
  opening = name;
  current = null;  // no edits while nothing is open
  markCurrent(name);
  setTool(null);
  $("message").textContent = "Opening the raw file…";
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
  for (const part of ["frame-tools", "crop-tools", "develop", "setup"]) $(part).inert = empty;
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
  const note = current.warning ?? (guessing ? "No clear film detected on this frame. Select custom area, or match base across the film roll from other frames." : "");
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
  // One choice per group: the tool being used, otherwise what the frame uses now.
  const cropMode = tool === "crop" || isCustomCrop(a.crop) ? "draw" : a.crop ? "frame" : "full";
  for (const m of ["full", "frame", "draw"]) $(`crop-${m}`).setAttribute("aria-pressed", String(cropMode === m));
  const customArea = tool === "area" || !!hints().analysis;
  $("area-auto").setAttribute("aria-pressed", String(!customArea));
  $("area-draw").setAttribute("aria-pressed", String(customArea));
  $("area-percent-row").hidden = customArea;
  showHolderLegend();
  const customBase = tool === "base" || !!hints().base;
  $("base-auto").setAttribute("aria-pressed", String(!customBase && !hints().base_rgb));
  $("roll-state").hidden = !!hints().base || !hints().base_rgb;
  showSetupNote();
  $("base-draw").setAttribute("aria-pressed", String(customBase));
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
  $(id).addEventListener("input", (e) => { if (current) { adj()[id] = Number(e.target.value); if (id === "straighten") straightenMoved(); changed(); } });
  $(id).addEventListener("dblclick", () => { if (current) { adj()[id] = 0; if (id === "straighten") straightenMoved(); changed(0); } });
}

// A new angle closes the areas, which no longer fit, but not the crop frame, since straightening and
// cropping go together. It measures again once it settles: on letting
// go of the slider, or after a short pause for arrow keys and typed values. Never while it's held.
const STRAIGHTEN_PAUSE_MS = 300;
let straightenTimer = null, straightenHeld = false, straightenUnmeasured = false;
function straightenMoved() {
  if (tool && tool !== "crop") setTool(null);
  straightenUnmeasured = true;
  clearTimeout(straightenTimer);
  if (!straightenHeld) straightenTimer = setTimeout(measureStraightened, STRAIGHTEN_PAUSE_MS);
}
async function measureStraightened() {
  if (!straightenUnmeasured || !current) return;
  // A custom area keeps its place on the scan while the picture turns under it.
  const custom = hints().base || hints().analysis;
  straightenUnmeasured = !(await measure(custom ? "Colors measured again. Custom areas stayed in place, so check them in Show areas." : "Colors measured again for the new angle."));
}
// Saved settings must hold what was measured for their angle, so saving and leaving wait for it.
async function measureSettled() {
  clearTimeout(straightenTimer);
  await measureStraightened();
  await lastMeasure;
}
$("straighten").addEventListener("pointerdown", (e) => { if (e.button === 0) straightenHeld = true; });
for (const end of ["pointerup", "pointercancel", "blur"]) {
  window.addEventListener(end, () => {
    if (!straightenHeld) return;
    straightenHeld = false;
    measureStraightened();
  });
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
  // The label's whole line edits the value: on a touch screen the box alone is a small target.
  range.closest(".slider").querySelector("label").addEventListener("click", (e) => {
    e.preventDefault();
    if (!box.disabled) box.focus();
  });
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

// Choosing anything else cancels an unfinished drawing. Show areas stays open to show the result.
// Only another crop choice cancels the crop frame: other tools apply it first.
function cancelDrawing() {
  if (tool && tool !== "show") setTool(null);
}
function finishDrawing() {
  applyOpenCrop();
  cancelDrawing();
}
$("crop-full").addEventListener("click", () => { if (current) { cancelDrawing(); setCrop(null); changed(0); } });
$("crop-frame").addEventListener("click", () => { if (current) { cancelDrawing(); setCrop({ ...current.settings.estimate.area }); changed(0); } });
$("crop-draw").addEventListener("click", () => { if (current && tool !== "crop") setTool("crop"); });
function toggleTool(t) {
  if (!current) return;
  applyOpenCrop();
  setTool(tool === t ? null : t);
}
$("area-draw").addEventListener("click", () => toggleTool("area"));
$("base-draw").addEventListener("click", () => toggleTool("base"));
$("show-areas").addEventListener("click", () => toggleTool("show"));
$("area-auto").addEventListener("click", () => {
  finishDrawing();
  if (current && hints().analysis) setHint("analysis", null);
});
$("base-auto").addEventListener("click", () => {
  finishDrawing();
  if (current && (hints().base || hints().base_rgb)) setHint("base", null);
});
$("base-roll").addEventListener("click", openRollFlow);
$("tool-cancel").addEventListener("click", () => setTool(null));
$("tool-done").addEventListener("click", () => { if (tool === "crop") applyCrop(); else setTool(null); });
document.addEventListener("keydown", (e) => {
  if ($("roll-flow").open || $("about").open) return;
  if (e.key === "Escape") {
    // One step back: out of a tool first, and only then away from a message.
    if (tool) setTool(null);
    else status();
    return;
  }
  // ⌘S or Ctrl+S saves, and never stops a save already running.
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    if (batch?.task !== "save" && !$("export").disabled) $("export").click();
    return;
  }
  if (tool === "crop") {
    // A button reached with the keyboard keeps its own Enter.
    if (e.key === "Enter" && !e.target.matches("button:not(.crop-h):focus-visible")) {
      e.preventDefault();
      applyCrop();
      return;
    }
    const step = CROP_NUDGE[e.key];
    if (step && !e.target.matches("input[type=range], input[type=text]")) {
      e.preventDefault();
      nudgeCrop(step, e.shiftKey ? 10 : 1, e.target.dataset?.edge);
      return;
    }
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
if (shell) document.addEventListener("contextmenu", (e) => { if (!e.target.matches("input[type=text]") && !e.target.closest("#note, #about")) e.preventDefault(); });

$("reset").addEventListener("click", () => {
  if (!current) return;
  for (const id of SLIDERS) adj()[id] = 0;
  adj().look = "neutral";
  changed(0);
  status("Tone and color reset.");
});

$("export").addEventListener("click", async () => {
  if (batch) {
    if (batch.task === "save") api("/api/export/cancel", {}).catch(() => {});
    return;
  }
  applyOpenCrop();
  const files = selected.size ? frames.map((f) => f.name).filter((n) => selected.has(n)) : current ? [current.file] : [];
  if (!files.length) return;
  try {
    await measureSettled();
    await rememberSettings();
    batch = await (await api("/api/export", { files })).json();
    updateSaveButton();
    followBatch();
  } catch (e) {
    status(`Could not save: ${e.message}`, { stay: true });
  }
});

// ---------- Matching the film base across a roll ----------
// Three steps, because this reaches past the frame on screen: say what will happen, read every
// frame without changing anything, then let the user choose which readings count.
let roll = null;  // { step, files, readings, ticked, voted }
const SOURCE = { picked: "Custom", detected: "Detected", unsure: "No clear film" };

function rollFrames() {
  const names = frames.map((f) => f.name);
  return selected.size ? names.filter((n) => selected.has(n)) : names;
}

function openRollFlow() {
  if (batch) { status("Wait for the current work to finish."); return; }
  finishDrawing();
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
    const frameWord = `${count} frame${count === 1 ? "" : "s"}`;
    const scope = count < frames.length ? `the selected ${frameWord}` : `all ${frameWord}`;
    body.innerHTML = `<p>In roll mode, all frames will share the same film base selection. Use it to unify the film base setting across the roll, or when some of the scans don't have a visible film edge.</p>
      <p>Reversal measures the film base on <b></b>. You can include both detected and custom film base selection on each frame, and pick which frames to use for the roll. You can verify and apply changes in the next step.</p>
      <p>Each frame keeps its own color analysis and Develop settings.</p>`;
    body.querySelector("b").textContent = scope;
    if (count < 2) body.insertAdjacentHTML("beforeend", `<p class="warn">Select at least two frames, or none to use the whole folder.</p>`);
    rollButtons("Cancel", `Measure ${count} frame${count === 1 ? "" : "s"}`, count >= 2);
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
  const verb = roll.step === "reading" ? "Measuring the film base on frame" : "Applying the film base selection to frame";
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
      ? `<span class="warn">No clear film detected on any frame.</span> Cancel, select a custom film base on a frame with clear film, and start again.`
      : "Selected frames determine film base settings for the entire roll. Deselect any that look incorrect."}</p>
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
    if (differs) note.title = "This film base differs from the rest of the roll. It may be misdetected, or from different film.";
    row.append(box, name, note);
    table.append(row);
  });
  table.scrollTop = scroll;
  const result = body.querySelector("#roll-result");
  const voters = roll.ticked.size;
  if (roll.voted?.base_rgb && voters) {
    const keeps = roll.readings.length - applied.length;
    result.textContent = `Film base from ${voters} frame${voters > 1 ? "s" : ""}.${keeps ? ` ${keeps} with a custom film base keep${keeps > 1 ? "" : "s"} it.` : ""}`;
  } else {
    result.textContent = "Select at least one frame.";
  }
  rollButtons("Cancel", `Apply to ${applied.length} frame${applied.length === 1 ? "" : "s"}`, voters > 0 && applied.length > 0);
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
    status(`Could not measure the roll: ${e.message}`, { stay: true });
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
    status(`Could not measure the roll: ${e.message}`, { stay: true });
  }
}

$("roll-go").addEventListener("click", () => {
  if (roll?.step === "intro") {
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
    status(`Could not get save progress: ${e.message}`, { stay: true });
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
      updateSaveButton();
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
      status(`Could not measure the roll: ${e.message}`, { stay: true });
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
  lastPath = finished.last_path;
  status(batchSummary(finished), { stay: true, reveal: !!(shell && lastPath) });
}

// The open frame's base and measurements changed on disk: read them back, and keep the slider
// moves made while measuring. A frame being opened already reads them.
async function reopenAfterRoll() {
  if (!current) return;
  const { file, settings: { adjust, custom_crop } } = current;
  current.remembered = JSON.stringify(current.settings);
  await open(file);
  if (current?.file === file) {
    current.settings.adjust = adjust;
    current.settings.custom_crop = custom_crop;
    changed(0);
  }
}

function rollSummary({ done, total, issues, canceled }) {
  const head = canceled ? "Stopped. No frame changed."
    : issues.length ? "Could not apply the film base selection to every frame."
    : `${total} frames now share one film base. Choose Detect to reset a frame to an individual setting.`;
  return [head, ...issues.map((i) => (i.file ? `${i.file}: ${i.message}` : i.message))].join("\n");
}

function batchSummary({ done, total, issues, canceled, last_path }) {
  const where = last_path ? ` to ${last_path.slice(0, last_path.lastIndexOf("/"))}` : "";
  const head = total === 1 && done === 0 ? "Could not save the DNG." : total === 1 && done === 1 ? `Saved ${last_path}` : `Saved ${done === total ? done : `${done} of ${total}`} DNGs${where}${canceled ? ", then stopped" : ""}.`;
  return [head, ...issues.map((i) => `${i.file}: ${i.message}`)].join("\n");
}

$("reveal").addEventListener("click", () => { if (shell && lastPath) shell.postMessage({ action: "reveal", path: lastPath }); });

async function setHint(key, rect) {
  if (!current) return;
  if (batch?.task === "roll") { status("Matching the film base. Try again when it's done."); return; }
  hints()[key] = rect;
  if (key === "base") hints().base_rgb = null;
  measure(key === "base" ? (rect ? "Custom film base set." : "Using current frame selection only.")
                         : (rect ? "Custom measured area set." : "Measuring the center of the scan."), true);
}

// Measures the frame as it's set now. A later call replaces an earlier one still on its way.
let latestMeasure = 0, lastMeasure = null;
function measure(done, clearsWarning = false) {
  lastMeasure = measureNow(done, clearsWarning);
  return lastMeasure;
}
// True once the frame holds the result, or a newer call has taken over.
async function measureNow(done, clearsWarning) {
  const file = current.file, id = ++latestMeasure;
  const areaBefore = current.settings.estimate.area;
  status("Measuring…");
  try {
    const { estimate, baseFound } = await (await api("/api/estimate", { file, hints: hints(), straighten: adj().straighten })).json();
    if (current?.file !== file || id !== latestMeasure) return true;
    current.settings.estimate = estimate;
    current.baseFound = baseFound;
    if (clearsWarning) current.warning = null;
    // A crop on the measured area follows it, checked now because the crop may have changed meanwhile.
    if (adj().crop && sameRect(adj().crop, areaBefore)) adj().crop = { ...estimate.area };
    if (cropDraft && sameRect(cropDraft, areaBefore)) cropDraft = { ...estimate.area };
    status(done);
  } catch (e) {
    if (current?.file !== file || id !== latestMeasure) return true;
    status(`Could not measure: ${e.message}`, { stay: true });
    return false;
  }
  changed(0);
  return true;
}

// ---------- Tools and overlay ----------
// Region tools show the whole scan (straightened, mirrored, rotated) with the areas drawn on it.
function setTool(t) {
  tool = t;
  document.body.dataset.tool = t ?? "";
  cropDraft = t === "crop" ? { ...cropToAdjust() } : null;
  cropDrag = null;
  // The crop frame shows once the entire scan is on screen.
  $("crop-layer").hidden = true;
  $("crop-layer").style.cursor = "";
  $("overlay").toggleAttribute("hidden", !t || t === "crop"); // SVG elements don't reflect .hidden
  $("toolbar").hidden = !t;
  $("tool-text").textContent = TOOL_TEXT[t] ?? "";
  $("tool-cancel").hidden = t === "show";
  $("tool-done").hidden = t === "area" || t === "base";
  $("tool-done").title = t === "crop" ? "Enter" : "Esc";
  $("tool-done").setAttribute("aria-keyshortcuts", t === "crop" ? "Enter" : "Escape");
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
const rectInView = (r, a) => rectFromPoints(scanToView([r.x0, r.y0], a), scanToView([r.x1, r.y1], a));
function svgRect(r, cls) {
  const v = rectInView(r, adj());
  return `<rect class="${cls}" x="${v.x0}" y="${v.y0}" width="${v.x1 - v.x0}" height="${v.y1 - v.y0}"/>`;
}
// What the analysis ignores as film holder, while Show areas is on. A custom area is measured as drawn.
const holderKey = () => [...current.settings.estimate.base_rgb, adj().straighten].join();
const holderKnown = () => current.holder?.key === holderKey();
function holderAreas() {
  return tool === "show" && !hints().analysis && holderKnown() ? current.holder.areas : [];
}
function showHolderLegend() {
  $("holder-legend").hidden = !holderAreas().length;
  if (tool !== "show") return;
  // Until the holder is known, keep the full text so it doesn't change under the reader.
  const noHolder = hints().analysis || (holderKnown() && !current.holder.areas.length);
  $("tool-text").textContent = noHolder ? "Showing the film base, measured area, and final crop." : TOOL_TEXT.show;
}
function holderShape() {
  const areas = holderAreas();
  if (!areas.length) return "";
  const a = adj();
  const d = areas.map((r) => {
    const v = rectInView(r, a);
    return `M${v.x0} ${v.y0}H${v.x1}V${v.y1}H${v.x0}Z`;
  }).join("");
  // Hatched, because a holder can show in any color, and a tint would read as one of the others.
  return `<pattern id="holder-hatch" width="0.016" height="0.016" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect class="hatch" width="0.006" height="0.016"/></pattern><path class="holder" d="${d}"/>`;
}
async function loadHolder() {
  const file = current.file, key = holderKey();
  if (current.holder?.key === key) return;
  const { base_rgb } = current.settings.estimate;
  try {
    const { areas } = await (await api("/api/holder", { file, base_rgb, straighten: adj().straighten })).json();
    if (current?.file === file) current.holder = { key, areas };
  } catch (e) {
    if (current?.file === file) status(`Could not find the film holder: ${e.message}`, { stay: true });
  }
}
function drawOverlay(drag) {
  if (!current || !tool) return;
  if (tool === "crop") {
    drawCropFrame();
    if ($("crop-layer").hidden) {
      $("crop-layer").hidden = false;
      $("crop-box").focus({ preventScroll: true });
    }
    return;
  }
  const e = current.settings.estimate;
  // The dashed measured area goes over the crop, so both show when they're the same rectangle.
  let s = holderShape() + (adj().crop ? svgRect(adj().crop, "crop") : "") + svgRect(e.area, "area") + svgRect(e.base_area, "base");
  if (drag) s += `<rect class="drag ${tool}" x="${drag.x0}" y="${drag.y0}" width="${drag.x1 - drag.x0}" height="${drag.y1 - drag.y0}"/>`;
  $("overlay").innerHTML = s;
  showHolderLegend();
}

let dragStart = null;
const eventPoint = (ev) => {
  const b = $("view").getBoundingClientRect();
  return [Math.min(1, Math.max(0, (ev.clientX - b.left) / b.width)), Math.min(1, Math.max(0, (ev.clientY - b.top) / b.height))];
};
$("overlay").addEventListener("pointerdown", (ev) => {
  if (!["area", "base"].includes(tool)) return;
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
  setTool(null);
  setHint(t === "area" ? "analysis" : "base", scan);
});

// ---------- Crop frame ----------
// The draft stays in scan coordinates so it follows a rotate or mirror. Drags work in the displayed
// view, where 0–1 spans the entire scan as shown.
const FULL_SCAN = { x0: 0, y0: 0, x1: 1, y1: 1 };
const CROP_MIN_PX = 40;      // so the frame's handles stay usable
const CROP_DRAW_MIN_PX = 4;  // a shorter drag is a click, which leaves the frame alone
const CROP_NUDGE = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
let cropDrag = null;  // { pointer, kind: "move" | "draw" | an edge, start: view point, from: view rect }

const isCustomCrop = (c) => c && !sameRect(c, current.settings.estimate.area);
// A custom crop stays with the frame when another crop replaces it, so Custom can bring it back.
function setCrop(crop) {
  if (isCustomCrop(adj().crop)) current.settings.custom_crop = adj().crop;
  adj().crop = crop;
}
function cropToAdjust() {
  const crop = adj().crop;
  return isCustomCrop(crop) ? crop : current.settings.custom_crop ?? crop ?? FULL_SCAN;
}
function applyCrop() {
  setCrop(sameRect(cropDraft, FULL_SCAN) ? null : cropDraft);
  setTool(null);
}
function applyOpenCrop() {
  if (tool === "crop" && current) applyCrop();
}
const clamp = (v, low, high) => Math.min(high, Math.max(low, v));
function cropView() {
  return rectInView(cropDraft, adj());
}
function setCropView(v) {
  const a = adj();
  cropDraft = rectFromPoints(viewToScan([v.x0, v.y0], a), viewToScan([v.x1, v.y1], a));
  drawCropFrame();
}
const cropFills = (v) => v.x1 - v.x0 > 0.999 && v.y1 - v.y0 > 0.999;
function drawCropFrame() {
  const v = cropView();
  for (const el of [$("crop-box"), $("crop-hole")]) {
    el.style.left = `${v.x0 * 100}%`;
    el.style.top = `${v.y0 * 100}%`;
    el.style.width = `${(v.x1 - v.x0) * 100}%`;
    el.style.height = `${(v.y1 - v.y0) * 100}%`;
  }
  $("crop-box").classList.toggle("fills", cropFills(v));
}
function cropMinSize() {
  const b = $("view").getBoundingClientRect();
  return [Math.min(1, CROP_MIN_PX / b.width), Math.min(1, CROP_MIN_PX / b.height)];
}
function movedRect(v, dx, dy) {
  const w = v.x1 - v.x0, h = v.y1 - v.y0;
  const x0 = clamp(v.x0 + dx, 0, 1 - w), y0 = clamp(v.y0 + dy, 0, 1 - h);
  return { x0, y0, x1: x0 + w, y1: y0 + h };
}
// `edge` names the sides that move: "n", "e", "s", "w", or a corner like "nw".
function resizedRect(v, edge, dx, dy, keepShape) {
  const [minW, minH] = cropMinSize();
  const r = { ...v };
  if (edge.includes("w")) r.x0 = clamp(v.x0 + dx, 0, v.x1 - minW);
  if (edge.includes("e")) r.x1 = clamp(v.x1 + dx, v.x0 + minW, 1);
  if (edge.includes("n")) r.y0 = clamp(v.y0 + dy, 0, v.y1 - minH);
  if (edge.includes("s")) r.y1 = clamp(v.y1 + dy, v.y0 + minH, 1);
  if (!keepShape || edge.length < 2) return r;
  // Grow the short side to match, then shrink both if that would leave the scan.
  const ratio = (v.x1 - v.x0) / (v.y1 - v.y0);
  let w = r.x1 - r.x0, h = r.y1 - r.y0;
  if (w / h > ratio) h = w / ratio; else w = h * ratio;
  const fit = Math.min(1, (edge.includes("w") ? v.x1 : 1 - v.x0) / w, (edge.includes("n") ? v.y1 : 1 - v.y0) / h);
  w *= fit; h *= fit;
  if (edge.includes("w")) r.x0 = v.x1 - w; else r.x1 = v.x0 + w;
  if (edge.includes("n")) r.y0 = v.y1 - h; else r.y1 = v.y0 + h;
  return r;
}
function drawnRect(p, q) {
  const [minW, minH] = cropMinSize();
  const r = rectFromPoints(p, q);
  if (r.x1 - r.x0 < minW) { r.x1 = Math.min(1, r.x0 + minW); r.x0 = r.x1 - minW; }
  if (r.y1 - r.y0 < minH) { r.y1 = Math.min(1, r.y0 + minH); r.y0 = r.y1 - minH; }
  return r;
}
// Arrows move the frame, or the edge or corner that has focus.
function nudgeCrop([dx, dy], px, edge) {
  const b = $("view").getBoundingClientRect();
  if (!b.width) return;
  const v = cropView(), x = dx * px / b.width, y = dy * px / b.height;
  setCropView(edge ? resizedRect(v, edge, x, y, false) : movedRect(v, x, y));
}
$("crop-layer").addEventListener("pointerdown", (ev) => {
  if (tool !== "crop" || ev.button !== 0 || cropDrag) return;
  ev.preventDefault();
  const from = cropView();
  const edge = ev.target.closest("[data-edge]")?.dataset.edge;
  // A frame filling the scan has nowhere to move, so dragging inside it draws a new one.
  const kind = edge ?? (ev.target.closest("#crop-box") && !cropFills(from) ? "move" : "draw");
  cropDrag = { pointer: ev.pointerId, kind, start: eventPoint(ev), from };
  // The capture decides the cursor, so keep the one for what was grabbed.
  $("crop-layer").style.cursor = getComputedStyle(ev.target).cursor;
  $("crop-layer").setPointerCapture(ev.pointerId);
  $("crop-box").focus({ preventScroll: true });
});
$("crop-layer").addEventListener("pointermove", (ev) => {
  if (cropDrag?.pointer !== ev.pointerId) return;
  const { kind, start, from } = cropDrag;
  const p = eventPoint(ev), dx = p[0] - start[0], dy = p[1] - start[1];
  if (kind === "move") setCropView(movedRect(from, dx, dy));
  else if (kind !== "draw") setCropView(resizedRect(from, kind, dx, dy, ev.shiftKey));
  else {
    const b = $("view").getBoundingClientRect();
    if (Math.max(Math.abs(dx) * b.width, Math.abs(dy) * b.height) >= CROP_DRAW_MIN_PX) setCropView(drawnRect(start, p));
    else setCropView(from);
  }
});
for (const type of ["pointerup", "pointercancel"]) {
  $("crop-layer").addEventListener(type, (ev) => {
    if (cropDrag?.pointer !== ev.pointerId) return;
    cropDrag = null;
    $("crop-layer").style.cursor = "";
  });
}

// ---------- Preview, fit and histogram ----------
async function refresh() {
  if (!current) return;
  if (rendering) { renderAgain = true; return; }
  rendering = true;
  const file = current.file;
  try {
    const full = !!tool;
    const res = await api("/api/preview", { file, settings: current.settings, full });
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
    if (tool === "show") {
      await loadHolder();
      if (file !== current?.file) return;
    }
    // A preview requested before a tool opened isn't the entire scan. Another follows it.
    if (full) drawOverlay();
    histogram(img, clipped);
  } catch (e) {
    if (file === current?.file) status(`Could not update the preview: ${e.message}`, { stay: true });
  } finally {
    rendering = false;
    if (renderAgain) { renderAgain = false; refresh(); }
  }
}

function fit() {
  const img = $("photo");
  if (!img.naturalWidth) return;
  const stage = $("stage");
  // A phone's stage is narrow, so it keeps a smaller margin around the image.
  const pad = stage.clientWidth < 600 ? 24 : 56;
  // Bars and messages pass over the image, except while cropping: the crop frame's handles
  // reach out past the image's edge and need to be free to drag, so the image keeps clear of
  // the crop bar. The bar grows on a touch screen, so this measures it. Twice, because the
  // image stays centered.
  const bar = $("toolbar");
  const reach = tool === "crop" ? parseFloat(getComputedStyle($("crop-box")).getPropertyValue("--edge-hit")) / 2 : 0;
  const padY = tool === "crop" ? Math.max(pad, 2 * ($("floats").offsetTop + bar.offsetTop + bar.offsetHeight + Math.max(10, reach))) : pad;
  const scale = Math.min((stage.clientWidth - pad) / img.naturalWidth, (stage.clientHeight - padY) / img.naturalHeight);
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
  $("clip-low").querySelector(".clip-pct").textContent = pct(low);
  $("clip-high").querySelector(".clip-pct").textContent = pct(high);
  $("clip-low").classList.toggle("over", low / n > 0.005);
  $("clip-high").classList.toggle("over", high / n > 0.005);
  $("clip-high").title = high / n > 0.005 && storedClipped < 0.0005 ? "The DNG keeps these. Lower Whites or use Flat to see them." : "";
  $("stored").hidden = storedClipped < 0.0005;
  $("stored").textContent = `${(storedClipped * 100).toFixed(2)}% is too bright to store. Lower Exposure.`;
  $("stored").title = `${(storedClipped * 100).toFixed(2)}% of pixels are brighter than the DNG can store. Lower Exposure to keep them.`;
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
