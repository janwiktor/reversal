// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

const $ = (id) => document.getElementById(id);
// A button's pressed or open state, as assistive technology and the stylesheet read it.
const press = (el, on) => el.setAttribute("aria-pressed", String(on));
const expand = (el, on) => el.setAttribute("aria-expanded", String(on));
// One choice per group, whose buttons are named `${group}-${choice}`.
const pressOne = (group, choices, chosen) => { for (const choice of choices) press($(`${group}-${choice}`), choice === chosen); };
const SLIDERS = ["exposure", "contrast", "highlights", "shadows", "whites", "blacks", "temperature", "tint", "saturation"];
const COLOR_SLIDERS = ["temperature", "tint", "saturation"];
const SAVE_SHORTCUT = navigator.platform.startsWith("Mac") ? "⌘S" : "Ctrl+S";
const LOOKS = ["neutral", "flat", "hard"];
const FILMS = ["color", "bw"];
const LIST = new Intl.ListFormat("en", { type: "conjunction" });
const TOOL_TEXT = {
  crop: "Drag to set crop for DNG export",
  area: "Drag over the picture only to set the measured area for conversion. Avoid film base, holder and sprocket holes",
  base: "Drag over clear, unexposed film, away from sprocket holes and edge markings",
};

let current = null;   // { file, settings }, and what the scan was read as
let tool = null;      // null | "crop" | "area" | "base" | "show" | "size", which Center size opens while it changes
let cropDraft = null;  // the open crop frame in scan coordinates; applied by Done or Enter
let timer = null;
let saveTimer = null;
let opening = null;       // the frame being loaded
// The frame on screen, or the one on its way there.
const frameNow = () => opening ?? current?.file;
let frames = [];          // what the frame list shows
const selected = new Set();
let lastChecked = null;   // for shift-click
let batch = null;         // the running export's progress
let lastPath = null;
let unfinished = [];      // frames a save was cut short before, as the folder lists them
let folderPath = "";      // the open folder, to name what's saved in it from there
let fullShown = false;  // the preview on screen is the entire scan, which areas are drawn on
// The last preview of each kind, as { key, url, clipped, job }, so going between the picture and
// its negative shows at once. `job` is what the engine was asked for.
const shots = { positive: null, negative: null };
let shownShot = null;  // the one on screen
let rendering = false;  // one preview at a time; changes meanwhile ask for one more
let renderAgain = false;

// In an app, the shell does what a page can't. The page sends it { action }: chooseFolder, busy
// with { busy }, reveal with { path }, export with { files }, and capabilities, which a shell
// may answer with what it does differently from the Mac app's:
//   export: it hands saved DNGs to other apps, where there's no file manager to show them in
//   rolls: chooseFolder opens its start screen, with the rolls it keeps, so the way there
//     stays in the bar
// reveal and export say where their button is, as { from: [x, y, width, height] }, for a shell
// that shows its choices next to it.
const shell = window.webkit?.messageHandlers?.reversal;
// Nothing, where there's no shell to hear it.
const toShell = (action, more) => shell?.postMessage({ action, ...more });
let shellCan = {};
// A shell that doesn't answer returns nothing here, and one that does returns a promise.
toShell("capabilities")?.then?.((can) => {
  shellCan = can ?? {};
  if (shellCan.export) $("reveal").textContent = "Export…";
  $("start-open").hidden = !shellCan.rolls;
  updateExportButton();
});
// An iPhone's number pads have no minus key, so its value fields that take one get the full
// keyboard, where numbers and the minus are a tap away. An iPad's number pad has one.
if (/iPhone|iPad|iPod|Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1 && Math.min(screen.width, screen.height) < 600) {
  for (const field of document.querySelectorAll("input.val")) {
    if (field.id === "area-percent-val") continue;
    field.inputMode = "text";
    // It's a number: no suggestions, capitals or corrections.
    field.autocapitalize = "off";
    field.spellcheck = false;
    field.setAttribute("autocorrect", "off");
  }
}
const placeOf = (button) => { const r = button.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
async function api(path, body) {
  const res = await fetch(path, body ? { method: "POST", keepalive: true, headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {});
  if (!res.ok) {
    let msg = `${res.status}`;
    try { msg = (await res.json()).error; } catch {}
    throw new Error(msg);
  }
  return res;
}
// A request whose answer is JSON.
const ask = async (path, body) => (await api(path, body)).json();
const clamp = (v, low, high) => Math.min(high, Math.max(low, v));
// A count with its word: 1 frame, 2 frames.
const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

const adj = () => current.settings.adjust;
const hints = () => current.settings.hints;
// The film type the frame converts as, or the one just chosen while it's measured for it.
const filmNow = () => current.pendingFilm ?? current.settings.estimate.film;

// ---------- Motion ----------
// Motion is brief and only shows what changed: a box that changes size eases there, and what
// appears or goes fades. The stylesheet animates what ends in one of its own states: a tool's bar,
// the image and the tools' drawings, the range warning over the side panel's histogram, dialogs,
// the frames column, and the room the image makes for a tool's bar. The script animates what
// depends on measuring, all through move(): the panels' edge, a row that comes or goes, a message's
// box, the floating histogram's box and the words that come and go in them. With reduced motion set, both change at
// once.
const stillMotion = matchMedia("(prefers-reduced-motion: reduce)");
const PHONE_WIDE = "not all and (min-width: 600px)";
const phoneLayout = matchMedia(PHONE_WIDE);
// A phone, and portrait: the frames list and the tools share one panel under the image, and the
// histogram floats over it.
const panelUnderImage = matchMedia(`${PHONE_WIDE}, (orientation: portrait)`);
const token = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
// A time token in milliseconds, whether written in s or ms.
const cssTime = (name) => { const v = token(name); return (parseFloat(v) || 0) * (v.endsWith("ms") ? 1 : 1000); };
const cssPx = (name) => parseFloat(token(name)) || 0;
const motion = (name) => ({ duration: cssTime(name), easing: token("--ease") });

// One motion per element and channel, such as "size" or "fade": a new one ends the last where it
// stands, and only the newest cleans up, since an ended one can still report finishing in the frame
// a newer one starts. Flags mark the element for the stylesheet while it moves, a tag says what
// kind of motion runs, and `after` runs as it ends, told whether it finished or was cut short.
// Keyframes end where the element rests without them, so a motion ended early, finished at once
// or skipped for reduced motion lands in the same place.
const motions = new WeakMap();
function move(el, channel, keyframes, options, { flags = [], tag = "", after } = {}) {
  endMotion(el, channel);
  if (stillMotion.matches) { after?.(true); return; }
  const runs = motions.get(el) ?? new Map();
  motions.set(el, runs);
  for (const flag of flags) el.dataset[flag] = "";
  const animation = el.animate(keyframes, options);
  runs.set(channel, { animation, flags, tag, after });
  animation.onfinish = () => { if (runs.get(channel)?.animation === animation) endMotion(el, channel, true); };
}
// Ends an element's motion on one channel, or on all of them: where it stands, or as finished.
function endMotion(el, channel, finished = false) {
  const runs = motions.get(el);
  for (const [name, run] of [...(runs ?? [])]) {
    if (channel && name !== channel) continue;
    runs.delete(name);
    run.animation.cancel();
    for (const flag of run.flags) if (![...runs.values()].some((r) => r.flags.includes(flag))) delete el.dataset[flag];
    run.after?.(finished);
  }
}
const motionTag = (el, channel) => motions.get(el)?.get(channel)?.tag;
// Fades an element in or out from wherever it is now, or in from `from`. Going out, it hides at the
// end, then `then` runs; brought back meanwhile, it stays.
function fadeIn(el, from = el.hidden ? 0 : Number(getComputedStyle(el).opacity)) {
  el.hidden = false;
  el.style.opacity = "";
  if (from < 1) move(el, "fade", { opacity: [from, 1] }, motion("--fade"));
  else endMotion(el, "fade");
}
function fadeOut(el, then) {
  if (el.hidden || motionTag(el, "fade") === "out") return;
  const from = Number(getComputedStyle(el).opacity);
  move(el, "fade", { opacity: [from, 0] }, motion("--fade"), {
    tag: "out",
    after: (finished) => { if (finished) { el.hidden = true; then?.(); } },
  });
}

// A panel that changes height slides there rather than jumping. The edge it shares with the image
// eases to its new place and the panel's contents ride that edge, so the image resizes smoothly
// with it. What a panel shows changes at once: only the edge moves.
function slidePanels(change, grid = panelUnderImage.matches ? document.body : $("bar")) {
  // On a phone and in portrait the panels sit under the image in the page's grid. Elsewhere only
  // the crop bar, under the top bar's row, changes height; the frames column slides by itself.
  const rows = () => getComputedStyle(grid).gridTemplateRows.split(" ").map(parseFloat);
  // From where the edge is now, mid-slide or not.
  const from = rows();
  endMotion(grid, "size");
  change();
  const to = rows();
  const n = to.length;
  if (from.length !== n || to.some(Number.isNaN) || from.every((h, i) => Math.abs(h - to[i]) < 1)) return;
  // The page's rows are the bar, the image, the panel rows and, on a phone, the tool bar. Only the
  // image's row moves: the panel rows under it fill from the top at their new heights and the last
  // takes what's left, so the panel's contents ride its top edge and the tool bar covers what
  // doesn't fit yet. The crop bar's two rows both move, and its contents ride their bottom edge.
  const last = phoneLayout.matches ? n - 2 : n - 1;
  const template = grid === document.body
    ? (end) => to.map((h, i) => (i === 1 ? `${(end ? to : from)[1]}px` : i === last ? "minmax(0px, 1fr)" : i > 1 && i < last ? `minmax(0px, ${h}px)` : `${h}px`)).join(" ")
    : (end) => (end ? to : from).map((h) => `${h}px`).join(" ");
  move(grid, "size", { gridTemplateRows: [template(false), template(true)] }, motion("--slide"), { flags: ["moving"] });
}

// A row that comes or goes eases its height, so what's under it slides there rather than jumps.
// `now` puts it in place at once.
function slideRow(row, show, now = false) {
  const going = motionTag(row, "size");
  if ((going ? going === "in" : !row.hidden) === show) return;
  const from = row.hidden ? 0 : row.getBoundingClientRect().height;
  endMotion(row, "size");
  row.hidden = false;
  const full = row.getBoundingClientRect().height;
  // Out of sight, in a panel that isn't shown, there's nothing to slide.
  if (now || !full) { row.hidden = !show; return; }
  const gap = parseFloat(getComputedStyle(row).marginTop) || 0;
  const part = (share) => ({ height: `${full * share}px`, marginTop: `${gap * share}px`, opacity: share });
  move(row, "size", [part(from / full), part(show ? 1 : 0)], motion("--slide"), {
    flags: ["moving"],
    tag: show ? "in" : "out",
    after: (finished) => { if (finished && !show) row.hidden = true; },
  });
}

// A double click, with a mouse or a finger. A touch screen sends none, so two short taps in one
// place count as one. `run` gets the event that completed it, for where it was.
const TAP_MS = 300, TAP_SLIP = 10, DOUBLE_TAP_APART = 24;
function onDouble(el, run) {
  el.addEventListener("dblclick", run);
  let down = null, last = null;
  const near = (e, to, px) => Math.hypot(e.clientX - to.x, e.clientY - to.y) < px;
  el.addEventListener("pointerdown", (e) => { down = e.pointerType === "mouse" ? null : { x: e.clientX, y: e.clientY, at: e.timeStamp }; });
  el.addEventListener("pointerup", (e) => {
    const tap = down && e.timeStamp - down.at < TAP_MS && near(e, down, TAP_SLIP) ? down : null;
    down = null;
    const second = tap && last && e.timeStamp - last.at < DOUBLE_CLICK_MS && near(e, last, DOUBLE_TAP_APART);
    last = second ? null : tap;
    if (second) run(e);
  });
}

// The page is an app's window, not a document, and keeps its size. A pinch on a trackpad comes as
// a wheel turn with Control held in some browsers and as a gesture in others.
addEventListener("wheel", (e) => { if (e.ctrlKey) e.preventDefault(); }, { passive: false });
for (const type of ["gesturestart", "gesturechange"]) addEventListener(type, (e) => e.preventDefault());

// ---------- Messages ----------
// Everything the app has to say floats at the foot of the image, one message at a time. A message
// that needs reading stays until dismissed or replaced; anything else fades once it has been read,
// later for longer words. A click or tap on it dismisses it, and anything longer than a line opens
// from Details.
const READ_MS = 3500, READ_MS_PER_CHAR = 40, READ_FREE_CHARS = 40, READ_MAX_MS = 8000;
// A click starts a dismissal that a second click, selecting a word, takes back.
const DOUBLE_CLICK_MS = 400;
let noteTimer = 0, noteStays = false, dismissTimer = 0;
let noteAction = null;  // what the message's own button does
// An offer to finish a save that was cut short, as { text, label, reveal }, until it's answered:
// taken, or put away by the user. Other messages come and go around it.
let offer = null;
let offerShown = false;  // the message on screen is the offer
// `action` is { label, run }: one thing to do about the message, which closes it.
function status(text = "", { stay = false, reveal = false, action = null } = {}) {
  if (offer && !action?.offer) {
    // A confirmation that would fade isn't worth losing the offer for, and nothing but the
    // user puts the offer away. A message that needs reading takes its place for a while.
    if (!text) { if (!offerShown) showOffer(); return; }
    if (!stay && !text.includes("\n")) return;
  }
  offerShown = !!action?.offer;
  const [head, ...rest] = text.split("\n");
  clearTimeout(noteTimer);
  clearTimeout(dismissTimer);
  noteStays = !!text && (stay || rest.length > 0);
  // No message hides the last one, which keeps its words while it fades.
  if (!text) { hideNote(); return; }
  showNote(() => {
    // A message that stays needs reading, so it interrupts; a confirmation waits its turn.
    $("note").setAttribute("role", noteStays ? "alert" : "status");
    $("note-words").textContent = head;
    $("note-more").textContent = rest.join("\n");
    showDetails(false);
    $("note-details").hidden = !rest.length;
    $("reveal").hidden = !reveal;
    noteAction = action?.run ?? null;
    $("note-action").hidden = !action;
    $("note-action").textContent = action?.label ?? "";
    // Without buttons, a message is its words alone.
    $("note").toggleAttribute("data-bare", !rest.length && !reveal && !action);
  });
  if (!noteStays) fadeLater(Math.min(READ_MAX_MS, READ_MS + READ_MS_PER_CHAR * Math.max(0, head.length - READ_FREE_CHARS)));
}
// What failed needs reading, so it stays.
const couldNot = (what, e) => status(`Could not ${what}: ${e.message}`, { stay: true });
// A message fades in, and one that follows it while it shows keeps the box: the words change at
// once and the box takes their height, then eases to their width. Growing, it uncovers the words
// in their final layout through a soft edge, which clears as it arrives.
function showNote(write) {
  const note = $("note");
  if (note.hidden || motionTag(note, "fade") === "out") {
    endMotion(note, "size");
    write();
    fadeIn(note);
    return;
  }
  const from = note.getBoundingClientRect().width;
  endMotion(note, "size");
  write();
  const to = note.getBoundingClientRect().width;
  if (Math.abs(from - to) < 1) return;
  const text = $("note-text"), words = $("note-words");
  if (to > from) {
    words.style.width = `${text.getBoundingClientRect().width}px`;
    const edge = token("--soft-edge");
    // The edge stays soft while the box grows, and sharpens as it settles.
    move(text, "edge", { "--edge": [edge, edge, "0px"], offset: [0, 0.7, 1] }, { ...motion("--slide"), easing: "linear" });
  }
  move(note, "size", { width: [`${from}px`, `${to}px`] }, motion("--slide"), {
    flags: ["moving"],
    after: () => { words.style.width = ""; endMotion(text, "edge"); },
  });
}
function hideNote() {
  endMotion($("note"), "size");
  fadeOut($("note"));
}
function fadeLater(ms) {
  noteTimer = setTimeout(() => {
    if ($("note").matches(":hover, :focus-within")) fadeLater(ms);
    else status();
  }, ms);
}
function showDetails(on) {
  $("note-more").hidden = !on;
  expand($("note-details"), on);
  // Reading the details keeps the message.
  if (on) { clearTimeout(noteTimer); noteStays = true; }
}
$("note-details").addEventListener("click", () => showDetails($("note-more").hidden));
// Anywhere but its buttons and Details, where words are read and selected. A double-click that
// selects a word, or a drag that selects some, keeps it.
$("note").addEventListener("click", (e) => {
  clearTimeout(dismissTimer);
  if (e.detail > 1 || e.target.closest("button, #note-more")) return;
  dismissTimer = setTimeout(() => { if (getSelection().isCollapsed) dismissNote(); }, DOUBLE_CLICK_MS);
});
// The user puts the message away. If it's the offer, that's its answer.
function dismissNote() {
  if (offerShown) forgetOffer();
  status();
}
// For the keyboard and screen readers, which can't click the box.
$("note-dismiss").addEventListener("click", dismissNote);
$("note-action").addEventListener("click", () => {
  const run = noteAction;
  // Once: the message fades for a moment after it's answered. The offer waits for other work.
  if (!run || (offerShown && batch)) return;
  noteAction = null;
  if (offerShown) offer = null;
  status();
  run();
});


// ---------- Files ----------
async function loadFiles() {
  const data = await ask("/api/files");
  folderPath = data.folder;
  unfinished = data.unfinished ?? [];
  // An offer has nothing left to save once the folder lists no such frames.
  if (!unfinished.length) withdrawOffer();
  $("folder").textContent = data.folder.split("/").filter(Boolean).pop() ?? data.folder;
  $("choose-folder").title = shell ? `${data.folder}\nOpen another folder…` : data.folder;
  frames = data.files;
  const names = new Set(frames.map((f) => f.name));
  for (const name of selected) if (!names.has(name)) selected.delete(name);
  const list = $("files");
  const scroll = list.scrollTop;
  list.replaceChildren(...frames.map(frameRow));
  list.scrollTop = scroll;
  if (!frames.length) $("message").textContent = "No raw files in this folder. Open a folder with NEF, CR2, CR3, ARW, RAF or DNG scans";
  markCurrent(frameNow());
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
  // Where the list takes the tools' place, a tap shows the frame and a tap on the frame that's
  // shown goes on to its tools: the list stays for looking through a roll, and gives way to work
  // on one frame.
  b.addEventListener("click", () => {
    if (panelUnderImage.matches && f.name === frameNow()) {
      // The row goes out of sight with the list, and keyboard focus goes to what brings it back.
      const focused = document.activeElement === b;
      slidePanels(() => showFrames(false));
      if (focused) $("frames-toggle").focus();
    } else open(f.name);
  });
  li.append(pick, b);
  showFrameState(f, state);
  return li;
}

function showFrameState(f, state) {
  const saving = batch?.task === "save" && batch.active.includes(f.name);
  state.textContent = saving ? "saving" : f.edited ? "edited" : f.saved ? "saved" : "";
  state.className = `state${f.edited && !saving ? " edited" : ""}${f.manualSetup ? " manual" : ""}`;
  state.title = f.manualSetup ? "Film type, film base or measured area set by hand" : f.edited ? "Changes not saved to a DNG yet" : "";
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
  showSelection();
}

// The checkboxes and what depends on them follow the selection.
function showSelection() {
  for (const box of document.querySelectorAll("#files input")) box.checked = selected.has(box.dataset.name);
  updateSaveButton();
}
// The selected frames, in the list's order.
const selectedNames = () => frames.map((f) => f.name).filter((name) => selected.has(name));

$("select-all-box").addEventListener("change", (e) => {
  selected.clear();
  if (e.target.checked) for (const f of frames) selected.add(f.name);
  showSelection();
});

// The open frame's place in the roll, or how many are selected.
let shownName = null;
// Where a shell hands DNGs to other apps, any saved frame can go at any time: the selected ones,
// or the open one. The button after a save only covers what that save wrote.
function exportable() {
  const wanted = selected.size ? frames.filter((f) => selected.has(f.name)) : frames.filter((f) => f.name === shownName);
  return wanted.filter((f) => f.saved).map((f) => f.name);
}
function updateExportButton() {
  $("export-dngs").hidden = !shellCan.export;
  $("export-dngs").disabled = !exportable().length;
}
$("export-dngs").addEventListener("click", (e) => {
  const files = exportable();
  if (files.length) toShell("export", { files, from: placeOf(e.currentTarget) });
});

function showCount() {
  updateExportButton();
  const at = frames.findIndex((f) => f.name === shownName), n = frames.length;
  $("frame-count").textContent = !n ? "" : selected.size ? `${selected.size} of ${n} selected` : at >= 0 ? `${at + 1} of ${n}` : plural(n, "frame");
}

// A batch's progress: the frame it's on, counting from 1, and the percent it has finished.
const frameInWork = ({ done = 0, failed = 0, total = 1 }) => Math.min(done + failed + 1, total);
const shareDone = ({ done = 0, failed = 0, total = 1 }) => 100 * (done + failed) / total;

// While saving, the button stops the save and fills as frames finish.
function updateSaveButton() {
  const button = $("export");
  toShell("busy", { busy: !!batch });
  showCount();
  const saving = batch?.task === "save";
  button.classList.toggle("cancel", saving);
  button.title = saving ? "Finish the frames being saved, then stop." : SAVE_SHORTCUT;
  if (saving) {
    const { total = 1 } = batch;
    button.textContent = total > 1 ? `Stop · ${frameInWork(batch)} of ${total}` : "Stop saving";
    button.style.setProperty("--done", `${shareDone(batch)}%`);
    // The button isn't announced as it changes, so progress is told separately.
    const told = `Saving ${frameInWork(batch)} of ${total}`;
    if ($("save-progress").textContent !== told) $("save-progress").textContent = told;
    button.disabled = false;
    return;
  }
  $("save-progress").textContent = "";
  button.textContent = selected.size ? `Save ${plural(selected.size, "DNG")}` : "Save DNG";
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
let framesColumn = true;
const framesShown = () => document.body.dataset.frames === "open";
function showFrames(on) {
  document.body.dataset.frames = on ? "open" : "closed";
  if (!panelUnderImage.matches) framesColumn = on;
  showFolds();
}
// What the panel under the image is showing: on a phone, Crop has a row of frame tools over it.
const panelParts = () => (framesShown() ? [$("roll")] : phoneLayout.matches && document.body.dataset.mode === "crop" ? [$("frame-tools"), $("crop-tools")] : [$("dock")]);
// What the frames button, the panel's handle and a phone's bar of tools say is shown. Under the
// image the list is in the panel, so it's shown only while the panel is, and the handle is the
// list's while the list is what the panel shows.
function showFolds() {
  const under = panelUnderImage.matches, list = under && framesShown();
  expand($("frames-toggle"), framesShown() && (dockShown() || !under));
  expand($("fold-panel"), dockShown());
  $("fold-panel").setAttribute("aria-controls", list ? "roll" : "dock");
  $("fold-panel").setAttribute("aria-label", list ? "Frames panel" : "Tools panel");
  // The frame tools over a phone's Crop panel keep their height, out of what the panel may have.
  const [fixed, scrolls] = panelParts();
  if (dockShown()) document.body.style.setProperty("--panel-fixed", `${scrolls ? fixed.getBoundingClientRect().height : 0}px`);
  pressModes();
}
// On a touch screen the panel under the image, whichever it shows, can be put away, to leave its
// room to the image. Beside the image the side panel always shows, whatever this says.
const dockShown = () => document.body.dataset.dock === "open";
function showDock(on) {
  // A value being typed in a panel that goes away is done: its keyboard would stay otherwise.
  if (!on && document.activeElement?.closest("#roll, #dock, #frame-tools, #crop-tools")) document.activeElement.blur();
  document.body.dataset.dock = on ? "open" : "closed";
  showFolds();
}
// On a phone the list takes the tools' place, so no group of tools reads as shown, and none
// while the panel is put away.
function pressModes() {
  const toolsShown = dockShown() && !framesShown();
  for (const button of document.querySelectorAll("#modes button")) {
    press(button, toolsShown && button.dataset.mode === document.body.dataset.mode);
  }
}
// The page starts on the list, which the handle says from the start.
showFolds();
// Under the image, the frame's name brings the list back from a panel that's put away.
function toggleFrames() {
  const unfold = panelUnderImage.matches && !dockShown();
  slidePanels(() => {
    showFrames(unfold || !framesShown());
    if (unfold) showDock(true);
  });
}
$("frames-toggle").addEventListener("click", toggleFrames);

// A drag on the panel's handle sets how tall the panel may be, from as tall as it is by itself
// down to PANEL_LEAST, and the image takes the rest. The height stays for whatever the panel
// shows next; a panel that needs less is as short as before. Dragged lower than that, the
// panel goes away, and comes back when the finger does. A tap puts it away or brings it back.
const PANEL_LEAST = 88;  // px: two rows of tools, less than which isn't worth keeping
const panelHeight = () => panelParts().reduce((sum, el) => sum + el.getBoundingClientRect().height, 0);
function limitPanel(height) {
  if (height == null) document.body.style.removeProperty("--panel-user");
  else document.body.style.setProperty("--panel-user", `${height}px`);
}
let panelDragged = -Infinity;  // when a drag last ended, since a click can follow it
let panelDrag = null;  // { pointer, y, from: the panel's height then, full: its height by itself, limit: the one set then, moved }
$("fold-panel").addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || panelDrag) return;
  endMotion(document.body, "size");
  const from = dockShown() ? panelHeight() : 0, limit = document.body.style.getPropertyValue("--panel-user"), shown = dockShown();
  // Its height by itself is measured open and with no limit, then put back before anything shows.
  document.body.dataset.dock = "open";
  limitPanel(null);
  const full = panelHeight();
  document.body.dataset.dock = shown ? "open" : "closed";
  if (limit) limitPanel(parseFloat(limit));
  panelDrag = { pointer: e.pointerId, y: e.clientY, from, full, limit: limit ? parseFloat(limit) : null, moved: false };
  $("fold-panel").setPointerCapture(e.pointerId);
});
$("fold-panel").addEventListener("pointermove", (e) => {
  if (panelDrag?.pointer !== e.pointerId) return;
  const height = panelDrag.from + panelDrag.y - e.clientY;
  if (!panelDrag.moved && Math.abs(e.clientY - panelDrag.y) < TAP_SLIP) return;
  panelDrag.moved = true;
  // A panel shorter than that by itself goes away only from under its own height.
  const show = height >= Math.min(PANEL_LEAST, panelDrag.full);
  if (show !== dockShown()) showDock(show);
  // Put away, it keeps the height it had, to come back to.
  // Dragged past its own height, a panel too short to have been limited keeps the limit set.
  limitPanel(!show ? panelDrag.limit : height < panelDrag.full - 1 ? height : panelDrag.limit > panelDrag.full ? panelDrag.limit : null);
});
for (const end of ["pointerup", "pointercancel", "lostpointercapture"]) {
  $("fold-panel").addEventListener(end, (e) => {
    if (panelDrag?.pointer !== e.pointerId) return;
    if (panelDrag.moved) panelDragged = e.timeStamp;
    panelDrag = null;
  });
}
// A drag's end is no tap, whenever the click for it comes.
$("fold-panel").addEventListener("click", (e) => {
  if (e.timeStamp - panelDragged < TAP_MS) return;
  slidePanels(() => showDock(!dockShown()));
});
panelUnderImage.addEventListener("change", () => {
  showFrames(panelUnderImage.matches ? !shownName : framesColumn);
  // With no frame open, the list under the image is all there is to work with.
  if (panelUnderImage.matches && !shownName) showDock(true);
});
// A motion measured for one layout finishes at once when the layout changes, so what was going
// goes, and the histogram's rows follow.
for (const layout of [phoneLayout, panelUnderImage]) {
  layout.addEventListener("change", () => {
    for (const el of [document.body, $("bar"), $("hist"), $("note")]) endMotion(el, "", true);
    showHistRows(histRows.clip, histRows.stored, { now: true });
  });
}
$("crop-open").addEventListener("click", () => slidePanels(() => {
  expand($("crop-open"), $("crop-open").getAttribute("aria-expanded") !== "true");
}, $("bar")));
for (const button of document.querySelectorAll("#modes button")) {
  button.addEventListener("click", () => slidePanels(() => {
    document.body.dataset.mode = button.dataset.mode;
    showFrames(false);
    showDock(true);
    // Each panel from its top: scrolled as the last one was, its heading would be out of view.
    $("dock").scrollTop = 0;
  }));
}

// In portrait the histogram floats over the image, and tapping it folds it to an icon. In
// landscape it has its own place and this button isn't shown.
$("hist-fold").addEventListener("click", () => foldHist(!$("hist").hasAttribute("data-folded")));
// Opening, the box grows from the icon's corner in one motion that reveals a part at a time: left
// and down to the histogram, then down to the clipping numbers, then to the range warning when
// there is one. Folding runs the same way back. The parts keep their open size and the box's
// edges clip them. The icon fades in once the box has folded.
// A short step still gets this much of the motion, so it reads as a step of its own.
const MIN_STEP_PX = 40;
function foldHist(folded) {
  const hist = $("hist"), body = $("hist-body"), icon = $("hist-fold").querySelector("svg");
  const box = (r) => ({ width: r.width, height: r.height });
  const from = box(hist.getBoundingClientRect());
  showHistRows(histRows.clip, histRows.stored, { now: true });
  // The stops are measured open: before folding, or once opened.
  if (!folded) hist.toggleAttribute("data-folded", false);
  const { bodyWidth, stops } = histStops();
  if (folded) hist.toggleAttribute("data-folded", true);
  const end = box(hist.getBoundingClientRect());
  expand($("hist-fold"), !folded);
  // From where the box is now, through the stops still ahead of it.
  const path = folded
    ? [from, ...[...stops].reverse().filter((s) => s.height < from.height - 0.5), end]
    : [from, ...stops.filter((s, i) => s.height > from.height + 0.5 || (i === 0 && s.width > from.width + 0.5))];
  // Time goes to each step in proportion to how far it moves the edges.
  const steps = path.slice(1).map((s, i) => Math.max(MIN_STEP_PX, Math.hypot(s.width - path[i].width, s.height - path[i].height)));
  const total = steps.reduce((a, b) => a + b, 0);
  let traveled = 0;
  const frames = path.map((s, i) => {
    if (i > 0) traveled += steps[i - 1];
    return { width: `${s.width}px`, height: `${s.height}px`, offset: total ? traveled / total : i / (path.length - 1) };
  });
  body.style.width = `${bodyWidth}px`;
  move(hist, "size", frames, motion("--slide-long"), {
    flags: ["moving", "folding"],
    tag: "fold",
    after: (finished) => {
      body.style.width = "";
      if (finished && folded) fadeIn(icon, 0);
    },
  });
}
// The open box's size at each part's foot, measured on the open layout.
function histStops() {
  const body = $("hist-body"), pad = cssPx("--hist-pad");
  const bodyWidth = body.offsetWidth, width = bodyWidth + 2 * pad;
  const top = body.getBoundingClientRect().top;
  const parts = [$("histogram"), $("clip"), $("stored")].filter((el) => !el.hidden);
  const stops = parts.map((el) => ({ width, height: el.getBoundingClientRect().bottom - top + 2 * pad }));
  return { bodyWidth, stops: stops.filter((s, i) => i === 0 || s.height > stops[i - 1].height + 0.5) };
}
// The floating histogram shows its clipping numbers and range warning only when there's something
// to report, so they come and go as sliders move. Coming, the box extends first and then the words
// fade in; going, the words fade out first and then the box shrinks, held at its height meanwhile.
// The side panel's histogram always shows its numbers, and its warning fades in over the
// histogram, in the stylesheet. `now` sets the rows at once, ending what's underway.
const histRows = { clip: false, stored: false };
// The floating histogram steps aside while a tool works on the image, or while no image shows.
const STEPPED_ASIDE = 'body:not([data-tool=""]) #hist, body:has(#view[hidden]) #hist';
function showHistRows(clip, stored, { now = false } = {}) {
  Object.assign(histRows, { clip, stored });
  const hist = $("hist"), floating = panelUnderImage.matches;
  const rows = [[$("clip"), clip || !floating], [$("stored"), stored]];
  const shown = floating && !hist.hasAttribute("data-folded") && hist.getClientRects().length > 0 && !hist.matches(STEPPED_ASIDE);
  if (now || !shown || motionTag(hist, "size") === "fold") {
    hist.style.height = "";
    for (const [el, on] of rows) {
      endMotion(el, "fade");
      el.style.opacity = "";
      el.hidden = !on;
    }
    return;
  }
  // Words that are wanted again come back from where their fade-out reached.
  for (const [el, on] of rows) if (on && motionTag(el, "fade") === "out") fadeIn(el);
  const going = rows.filter(([el, on]) => !on && !el.hidden).map(([el]) => el);
  // Words still waiting unseen go at once; only shown ones fade.
  const dropped = going.filter((el) => el.style.opacity === "0");
  for (const el of dropped) { el.hidden = true; el.style.opacity = ""; }
  const fading = going.filter((el) => !el.hidden);
  if (fading.length) {
    // The box stops where it is and holds there while the words go.
    const height = hist.getBoundingClientRect().height;
    endMotion(hist, "size");
    hist.style.height = `${height}px`;
    for (const el of fading) fadeOut(el, () => showHistRows(histRows.clip, histRows.stored));
    return;
  }
  // Words wait unseen until the box has room for them, including any whose wait was cut short.
  const coming = rows.filter(([el, on]) => on && el.hidden).map(([el]) => el);
  const waiting = rows.filter(([el, on]) => on && !el.hidden && el.style.opacity === "0").map(([el]) => el);
  // Nothing new: a running grow or shrink carries on, and a box at rest stays.
  const resizing = motionTag(hist, "size") === "rows";
  if (!dropped.length && !coming.length && (resizing || (!waiting.length && !hist.style.height))) return;
  const from = hist.getBoundingClientRect().height;
  endMotion(hist, "size");
  hist.style.height = "";
  for (const el of coming) { el.style.opacity = "0"; el.hidden = false; }
  const appear = [...waiting, ...coming];
  const to = hist.getBoundingClientRect().height;
  if (Math.abs(from - to) < 0.5) { appear.forEach(fadeIn); return; }
  move(hist, "size", { height: [`${from}px`, `${to}px`] }, motion("--slide"), {
    flags: ["moving"],
    tag: "rows",
    after: (finished) => { if (finished) appear.forEach(fadeIn); },
  });
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
  const state = await ask("/api/settings", { file, settings, adjust_only: rolling });
  frame.remembered = json;
  const f = frames.find((x) => x.name === file);
  if (f) { Object.assign(f, state); showFrameStates(); }
}

function rememberSoon() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => rememberSettings().catch((e) => couldNot("save settings", e)), 300);
}

async function open(name) {
  applyOpenCrop();
  try {
    await measureSettled();
    await rememberSettings(true);
  } catch (e) {
    couldNot("save settings, so this frame stays open", e);
    return;
  }
  clearTimeout(timer);
  clearTimeout(areaTimer);
  clearTimeout(straightenTimer);
  straightenUnmeasured = centerSizeUnmeasured = false;
  negativeKept = false;
  opening = name;
  current = null;  // no edits while nothing is open
  markCurrent(name);
  setTool(null);
  $("message").textContent = "Opening the raw file…";
  $("message").hidden = false;
  $("view").hidden = true;
  showDetail();
  setBusy(true);
  try {
    const data = await ask(`/api/load?file=${encodeURIComponent(name)}`);
    if (opening !== name) return;  // another frame was chosen meanwhile
    current = { file: name, settings: data.settings, defaultAreaPercent: data.defaultAreaPercent, warning: data.warning, baseFound: data.baseFound, filmDetected: data.filmDetected };
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
  for (const part of ["frame-tools", "view-tools", "crop-tools", "develop", "setup"]) $(part).inert = empty;
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
  const note = current.warning ?? (guessing ? "No clear film detected. Draw it with Custom, or take it from other frames with Match film base" : "");
  $("setup-note").textContent = note;
  $("setup-note").hidden = !note;
}

let syncedFrame = null;  // the frame the controls last showed
function syncControls() {
  const film = filmNow();
  // The panel under the image is as tall as what it shows, so its edge slides when the film type
  // changes that, to where everything else that changes with it leaves the panel.
  if (document.body.dataset.film !== film) slidePanels(() => showControls(film));
  else showControls(film);
}
function showControls(film) {
  const a = adj();
  // Black and white has only Tone to develop. On a phone, its Color panel gives way to Tone.
  document.body.dataset.film = film;
  if (film === "bw" && document.body.dataset.mode === "color") {
    document.body.dataset.mode = "tone";
    pressModes();
  }
  pressOne("film", FILMS, film);
  const chosen = !!hints().film;
  const reads = current.filmDetected === "bw" ? "black and white" : "color";
  // A label's dot says its choice departs from what's detected, in the color its area is drawn in.
  $("l-film").classList.toggle("changed", chosen);
  $("l-base").classList.toggle("changed", !!(hints().base || hints().base_rgb));
  $("l-area").classList.toggle("changed", !!hints().analysis || hints().area_percent != null);
  $("crop-label").classList.toggle("changed", !!a.crop);
  $("film-seg").title = chosen
    ? `Chosen for this frame. The scan reads as ${reads}; choose that to go back to detecting.`
    : "Detected from the scan. Choose the other type to convert this frame as that instead.";
  for (const id of [...SLIDERS, "straighten"]) showValue(id, a[id]);
  pressOne("look", LOOKS, a.look);
  const percent = hints().area_percent ?? current.defaultAreaPercent;
  showValue("area-percent", percent);
  press($("flip"), a.flip_h);
  // One choice per group: the tool being used, otherwise what the frame uses now.
  const cropMode = tool === "crop" || isCustomCrop(a.crop) ? "draw" : a.crop ? "frame" : "full";
  pressOne("crop", ["full", "frame", "draw"], cropMode);
  const customArea = tool === "area" || !!hints().analysis;
  pressOne("area", ["auto", "draw"], customArea ? "draw" : "auto");
  // Another frame shows its own at once; a choice made on this one slides the row in or out.
  slideRow($("area-percent-row"), !customArea, syncedFrame !== current.file);
  syncedFrame = current.file;
  showLegend();
  const customBase = tool === "base" || !!hints().base;
  press($("base-auto"), !customBase && !hints().base_rgb);
  $("roll-state").hidden = !!hints().base || !hints().base_rgb;
  showSetupNote();
  press($("base-draw"), customBase);
  press($("show-areas"), tool === "show");
  press($("negative"), negativeNow());
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
  const reset = () => { if (current) { adj()[id] = 0; if (id === "straighten") straightenMoved(); changed(0); } };
  onDouble($(id), reset);
}

// A new angle closes the areas, which no longer fit, but not the crop frame, since straightening and
// cropping go together. It measures again once it settles: on letting
// go of the slider, or after a short pause for arrow keys and typed values. Never while it's held.
const SETTLE_PAUSE_MS = 300;
// Whether a pointer holds a slider, with `release` run when it lets go.
function holdable(range, release) {
  let held = false;
  range.addEventListener("pointerdown", (e) => { if (e.button === 0) held = true; });
  const end = () => {
    if (!held) return;
    held = false;
    release();
  };
  for (const type of ["pointerup", "pointercancel", "blur"]) window.addEventListener(type, end);
  // A slider says so itself when a drag commits its value, should the pointer's end go unheard.
  range.addEventListener("change", end);
  return () => held;
}
let straightenTimer = null, straightenUnmeasured = false;
function straightenMoved() {
  if (tool && tool !== "crop") setTool(null);
  straightenUnmeasured = true;
  clearTimeout(straightenTimer);
  if (!straightenHeld()) straightenTimer = setTimeout(measureStraightened, SETTLE_PAUSE_MS);
}
async function measureStraightened() {
  if (!straightenUnmeasured || !current) return;
  // A custom area keeps its place on the scan while the picture turns under it.
  const custom = hints().base || hints().analysis;
  // Cleared first, so an angle set while this one is measured still counts as unmeasured.
  straightenUnmeasured = false;
  if (!(await measure(custom ? "Colors measured again. Check custom areas in Show areas" : "Colors measured again for the new angle"))) straightenUnmeasured = true;
}
// Saved settings must hold what was measured for their angle, so saving and leaving wait for it.
async function measureSettled() {
  clearTimeout(straightenTimer);
  await measureStraightened();
  await measureCenterSize();
  await lastMeasure;
}
const straightenHeld = holdable($("straighten"), measureStraightened);

for (const look of LOOKS) {
  $(`look-${look}`).addEventListener("click", () => { if (current) { adj().look = look; changed(0); } });
}

// While Center size changes, the entire scan shows with the area it would measure, dashed, and
// nothing is measured. It measures once the size settles, as Straighten does: on letting go of
// the slider, or after a short pause for arrow keys and typed values. Then the picture comes back.
let areaTimer = null, centerSizeUnmeasured = false;
const centerArea = () => {
  const margin = (1 - (hints().area_percent ?? current.defaultAreaPercent) / 100) / 2;
  return { x0: margin, y0: margin, x1: 1 - margin, y1: 1 - margin };
};
// `settled` is a size set in one step, as a reset is: it's measured at once, with nothing to show
// meanwhile.
function setAreaPercent(percent, settled = false) {
  if (!current) return;
  // The default is no choice at all, however it was reached.
  hints().area_percent = percent === current.defaultAreaPercent ? null : percent;
  centerSizeUnmeasured = true;
  syncControls();
  if (settled) { measureCenterSize(); return; }
  // Show areas already draws the area. Any other tool gives way, a crop frame applied first.
  if (tool !== "show" && tool !== "size") {
    applyOpenCrop();
    setTool("size");
  } else if (fullShown) drawOverlay();
  clearTimeout(areaTimer);
  if (!centerSizeHeld()) areaTimer = setTimeout(measureCenterSize, SETTLE_PAUSE_MS);
}
async function measureCenterSize() {
  clearTimeout(areaTimer);
  if (!centerSizeUnmeasured || !current) return;
  centerSizeUnmeasured = false;
  // The picture comes back at once, and its colors follow when they're measured.
  if (tool === "size") setTool(null);
  // A measurement that failed is still owed, so saving or leaving the frame tries it again. One
  // put off for a roll isn't: the roll measures the frame itself.
  if ((await setHint("analysis", null)) === false) centerSizeUnmeasured = true;
}
const centerSizeHeld = holdable($("area-percent"), measureCenterSize);
$("area-percent").title = "Double-click to reset";
$("area-percent").addEventListener("input", (e) => setAreaPercent(Number(e.target.value)));
onDouble($("area-percent"), () => setAreaPercent(null, true));

// A value box drives its slider, so a typed number goes the same way as a drag.
for (const id of [...SLIDERS, "straighten", "area-percent"]) {
  const range = $(id), box = $(`${id}-val`);
  // The label's whole line edits the value: on a touch screen the box alone is a small target.
  range.closest(".slider").querySelector("label").addEventListener("click", (e) => {
    e.preventDefault();
    if (!box.disabled) box.focus();
  });
  const set = (v) => {
    const clamped = clamp(v, Number(range.min), Number(range.max));
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
for (const film of FILMS) $(`film-${film}`).addEventListener("click", () => setFilm(film));
$("tool-cancel").addEventListener("click", () => setTool(null));
$("tool-done").addEventListener("click", () => { if (tool === "crop") applyCrop(); else setTool(null); });
document.addEventListener("keydown", (e) => {
  if ($("roll-flow").open || $("about").open) return;
  if (e.key === "Escape") {
    // One step back: out of a tool first, and only then away from a message.
    if (tool) setTool(null);
    else dismissNote();
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
  if (e.key.toLowerCase() === "n" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.target.matches("input[type=text]")) {
    if (!e.repeat) pressNegative();
    return;
  }
  // Up and down walk the frames, unless a slider or text box has the keys.
  if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.target.matches("input[type=range], input[type=text]")) {
    e.preventDefault();
    const at = frames.findIndex((f) => f.name === frameNow());
    const next = frames[e.key === "ArrowUp" ? Math.max(at - 1, 0) : Math.min(at + 1, frames.length - 1)];
    if (next && next.name !== frameNow()) open(next.name);
  }
});
// A Mac sends no keyup for a letter let go while Command is held, so Command's own counts.
document.addEventListener("keyup", (e) => { if (e.key.toLowerCase() === "n" || e.key === "Meta") releaseNegative(); });
// In the app, a right-click shouldn't offer a web page's Reload.
if (shell) document.addEventListener("contextmenu", (e) => { if (!e.target.matches("input[type=text]") && !e.target.closest("#note, #about")) e.preventDefault(); });

$("reset").addEventListener("click", () => {
  if (!current) return;
  // Black and white keeps its hidden color settings, for if it's switched back to color.
  const bw = filmNow() === "bw";
  for (const id of SLIDERS) if (!(bw && COLOR_SLIDERS.includes(id))) adj()[id] = 0;
  adj().look = "neutral";
  changed(0);
  status(bw ? "Tone reset" : "Tone and color reset");
});

$("export").addEventListener("click", async () => {
  if (batch) {
    if (batch.task === "save") api("/api/export/cancel", {}).catch(() => {});
    return;
  }
  applyOpenCrop();
  const files = selected.size ? selectedNames() : current ? [current.file] : [];
  if (files.length) save(files);
});

async function save(files) {
  if (batch) return;
  try {
    await measureSettled();
    await rememberSettings();
    batch = await ask("/api/export", { files });
    // A save lists its own frames, in place of what an earlier one left.
    unfinished = [];
    withdrawOffer();
    updateSaveButton();
    followBatch();
  } catch (e) {
    couldNot("save", e);
  }
}

// A save that was cut short, by the app being closed or the device needing a rest, left frames
// unsaved, and the folder lists them. They're offered until the user takes the offer, puts it
// away, or saves something else: the list is only dropped then, so that closing the app or
// leaving the folder with the offer on screen loses nothing.
function offerRest(text, label, reveal = false) {
  if (!unfinished.length) return false;
  offer = { text, label, reveal };
  showOffer();
  return true;
}
function showOffer() {
  const files = unfinished;
  status(offer.text, { stay: true, reveal: offer.reveal, action: { label: offer.label, offer: true, run: () => save(files) } });
}
// The offer goes without an answer: there's nothing left for it to save.
function withdrawOffer() {
  if (!offer) return;
  offer = null;
  if (offerShown) status();
}
function forgetOffer() {
  offer = null;
  unfinished = [];
  api("/api/export/forget", {}).catch(() => {});
}

// ---------- Matching the film base across a roll ----------
// Three steps, because this reaches past the frame on screen: say what will happen, read every
// frame without changing anything, then let the user choose which readings count.
let roll = null;  // { step, files, readings, issues, ticked, voted }
const SOURCE = { picked: "Custom", detected: "Detected", unsure: "No clear film" };

function rollFrames() {
  return selected.size ? selectedNames() : frames.map((f) => f.name);
}

function openRollFlow() {
  if (batch) { status("Wait for the save to finish"); return; }
  finishDrawing();
  roll = { step: "intro", files: rollFrames() };
  showRollFlow();
  $("roll-flow").showModal();
}

function closeRollFlow() {
  roll = null;
  if ($("roll-flow").open) $("roll-flow").close();
}
function rollFailed(e) {
  closeRollFlow();
  couldNot("measure the roll", e);
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
    const frameWord = plural(count, "frame");
    const scope = count < frames.length ? `the selected ${frameWord}` : `all ${frameWord}`;
    body.innerHTML = `<p>Give every frame the same film base, taken from several frames. Use it for a consistent roll, or when some scans show no clear film edge.</p>
      <p>Reversal first measures the film base on <b></b>, without changing them. Then you choose which results to use, and apply.</p>
      <p>It does not change custom film base settings, color measurement, and Develop settings of each frame.</p>`;
    body.querySelector("b").textContent = scope;
    if (count < 2) body.insertAdjacentHTML("beforeend", `<p class="warn">Select at least two frames, or none to use the whole folder.</p>`);
    rollButtons("Cancel", `Measure ${frameWord}`, count >= 2);
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
  const verb = roll.step === "reading" ? "Measuring the film base on frame" : "Applying the film base to frame";
  progress.textContent = `${verb} ${frameInWork(batch)} of ${batch.total}…`;
  progress.nextElementSibling.firstElementChild.style.width = `${shareDone(batch)}%`;
}

function showRollReview() {
  const body = $("roll-body");
  const scroll = body.querySelector("#roll-table")?.scrollTop ?? 0;
  const applied = roll.readings.filter((r) => r.source !== "picked");
  const lost = roll.issues.length ? `<p class="warn" id="roll-issues"></p>` : "";
  const noClear = roll.readings.every((r) => r.source === "unsure");
  body.innerHTML = `<p>${noClear
      ? `<span class="warn">No clear film detected on any frame.</span> Cancel, set a custom film base on a frame that shows clear film, and start again.`
      : "Selected frames set the film base for the roll. Deselect any that look wrong."}</p>
    ${lost}<div id="roll-table"></div><div id="roll-result"></div>`;
  if (lost) body.querySelector("#roll-issues").textContent = issueLines(roll.issues).join("\n");
  const table = body.querySelector("#roll-table");
  roll.readings.forEach((r) => {
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
    result.textContent = `Film base from ${plural(voters, "frame")}.${keeps ? ` ${keeps} with a custom film base keep${keeps > 1 ? "" : "s"} it.` : ""}`;
  } else {
    result.textContent = "Select at least one frame.";
  }
  rollButtons("Cancel", `Apply to ${plural(applied.length, "frame")}`, voters > 0 && applied.length > 0);
}

// The median and what strays from it come from the engine, like every other measurement.
async function voteRoll() {
  const review = roll;
  const voters = review.readings.filter((r) => review.ticked.has(r.file));
  if (!voters.length) { review.voted = null; showRollReview(); return; }
  try {
    const { base_rgb, differs } = await ask("/api/roll/vote", { bases: voters.map((r) => r.base_rgb), readings: review.readings.map((r) => r.base_rgb) });
    if (roll !== review) return;
    review.voted = { base_rgb, differs: Object.fromEntries(review.readings.map((r, i) => [r.file, differs[i]])) };
    showRollReview();
  } catch (e) {
    rollFailed(e);
  }
}

async function startRollTask(step, url, body) {
  roll.step = step;
  showRollFlow();
  try {
    await rememberSettings();
    batch = await ask(url, body);
    updateSaveButton();
    followBatch();
  } catch (e) {
    rollFailed(e);
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
    // Measuring a roll saves nothing, so stopping it leaves the frames an earlier save didn't
    // reach as they are listed.
    api("/api/export/cancel", { later: true }).catch(() => {});
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
    status("Stopped. No frame changed");
    return;
  }
  const readings = await ask("/api/roll/readings");
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
    batch = await ask("/api/export/status");
  } catch (e) {
    batch = null;
    updateSaveButton();
    couldNot("get save progress", e);
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
  // A sharper part put off for the save can come now.
  wantDetail();
  if (finished.task === "roll-read") {
    updateSaveButton();
    rollReadingsDone(finished).catch(rollFailed);
    return;
  }
  if (inRoll) { $("roll-cancel").disabled = false; closeRollFlow(); }
  await loadFiles();
  if (finished.task === "roll") {
    await reopenAfterRoll();
    status(rollSummary(finished));
    return;
  }
  // Where DNGs leave through the shell, the frames stay selected: Export… is the next step.
  if (finished.done === finished.total && !shellCan.export) selected.clear();
  showSelection();
  lastPath = finished.last_path;
  const reveal = !!(shell && lastPath);
  // Stopped for now, not by the user: the rest is still wanted.
  if (finished.canceled && offerRest(batchSummary(finished), "Save the rest", reveal)) return;
  // Like any message, a save's result fades unless it has problems to read. Where its button
  // is the way out for the DNG, it waits to be used or put away.
  status(batchSummary(finished), { reveal, stay: reveal && !!shellCan.export });
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

function rollSummary({ total, issues, canceled }) {
  const head = canceled ? "Stopped. No frame changed"
    : issues.length ? "Could not apply the film base to every frame"
    : `${total} frames now share one film base. Choose Detect on a frame to measure its own`;
  return [head, ...issueLines(issues)].join("\n");
}

// A file in the open folder goes by its path from there: the folder is named on screen, and an
// app's own folder has a path that tells nobody anything.
function inFolder(path) {
  return folderPath && path.startsWith(`${folderPath}/`) ? path.slice(folderPath.length + 1) : path;
}

// What went wrong, a line for each frame, or for the whole task where no frame is named.
const issueLines = (issues) => issues.map((i) => (i.file ? `${i.file}: ${i.message}` : i.message));

function batchSummary({ done, total, issues, canceled, last_path }) {
  const saved = last_path && inFolder(last_path);
  // Where DNGs leave through the shell, the folder they're kept in is no news: the name will do.
  const name = saved && shellCan.export ? saved.slice(saved.lastIndexOf("/") + 1) : saved;
  const where = saved && !shellCan.export ? ` to ${saved.slice(0, saved.lastIndexOf("/"))}` : "";
  const head = total === 1 && done === 0 ? "Could not save the DNG" : total === 1 && done === 1 ? `Saved ${name}` : `Saved ${done === total ? done : `${done} of ${total}`} DNGs${where}${canceled ? ", then stopped" : ""}`;
  return [head, ...issueLines(issues)].join("\n");
}

$("reveal").addEventListener("click", (e) => { if (lastPath) toShell("reveal", { path: lastPath, from: placeOf(e.currentTarget) }); });

// Measuring a roll reads every frame's settings, so they wait until it's done.
function rollBusy() {
  if (batch?.task !== "roll") return false;
  status("Matching the film base. Try again when it's done");
  return true;
}

async function setHint(key, rect) {
  if (!current) return;
  if (rollBusy()) return;
  hints()[key] = rect;
  if (key === "base") hints().base_rgb = null;
  return measure(key === "base" ? (rect ? "Custom film base set" : "Film base set to Detect")
                         : (rect ? "Custom measured area set" : "Measured area set to Center"), true);
}

// The detected type is no choice at all, so choosing it again goes back to detecting.
// A choice shows at once and holds while it's measured, so a second click compares with it.
function setFilm(film) {
  if (!current || filmNow() === film) return;
  if (rollBusy()) return;
  hints().film = film === current.filmDetected ? null : film;
  current.pendingFilm = film;
  syncControls();
  measure(film === "bw" ? "Processing as black and white" : "Processing as color");
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
    // Measuring again keeps the film type, so moving an area can't switch the conversion.
    const film = filmNow();
    const { estimate, baseFound, filmDetected } = await ask("/api/estimate", { file, hints: hints(), straighten: adj().straighten, film });
    if (current?.file !== file || id !== latestMeasure) return true;
    current.settings.estimate = estimate;
    current.baseFound = baseFound;
    current.filmDetected = filmDetected;
    current.pendingFilm = null;
    if (clearsWarning) current.warning = null;
    // A crop on the measured area follows it, checked now because the crop may have changed meanwhile.
    if (adj().crop && sameRect(adj().crop, areaBefore)) adj().crop = { ...estimate.area };
    if (cropDraft && sameRect(cropDraft, areaBefore)) cropDraft = { ...estimate.area };
    status(done);
  } catch (e) {
    if (current?.file !== file || id !== latestMeasure) return true;
    // The frame still converts as it did, so show that.
    current.pendingFilm = null;
    syncControls();
    couldNot("measure", e);
    return false;
  }
  changed(0);
  return true;
}

// ---------- The original negative ----------
// The scan itself, in place of its positive, framed the same way. Over the picture it's a look
// to compare with, so it shows while its button or N is held. Where areas are shown or drawn on
// the entire scan, it's what they're judged on, so there it's a choice that stays, and comes back
// with the next area tool on any frame.
const AREA_TOOLS = ["show", "area", "base"];
const onAreas = () => AREA_TOOLS.includes(tool);
let negativeOnAreas = false;  // the choice where areas show
let negativeKeyed = false;    // N is held
let negativeKept = false;     // turned on from the keyboard, where a button can't be held; until the frame changes
const negativeNow = () => (onAreas() ? negativeOnAreas : negativeHeld() || negativeKeyed || negativeKept);
function showNegative() {
  if (!current) return;
  syncControls();
  clearTimeout(timer);
  refresh();
}
function pressNegative() {
  if (onAreas()) negativeOnAreas = !negativeOnAreas;
  else [negativeKeyed, negativeKept] = [true, false];
  showNegative();
}
function releaseNegative() {
  if (!negativeKeyed) return;
  negativeKeyed = false;
  showNegative();
}
const negativeHeld = holdable($("negative"), showNegative);
$("negative").addEventListener("pointerdown", (e) => { if (e.button === 0) showNegative(); });
$("negative").addEventListener("click", (e) => {
  // A pointer's click ends a hold, which has shown and hidden the negative already: all it
  // has left to do is let go of one the keyboard kept.
  if (onAreas()) negativeOnAreas = !negativeOnAreas;
  else if (e.detail === 0) negativeKept = !negativeKept;
  else if (negativeKept) negativeKept = false;
  else return;
  showNegative();
});
addEventListener("blur", releaseNegative);

// ---------- Tools and overlay ----------
// Region tools show the whole scan (straightened, mirrored, rotated) with the areas drawn on it.
function setTool(t) {
  // A tool opens on the entire scan, which is what it works on. From another tool, the view
  // stays where it was moved to.
  if (t && !tool) zoomOut();
  tool = t;
  document.body.dataset.tool = t ?? "";
  cropDraft = t === "crop" ? { ...cropToAdjust() } : null;
  cropDrag = null;
  dragStart = null;
  // Center size shows its area on the entire scan, so the area is in view.
  if (t === "size") zoomOut();
  // The crop frame shows once the entire scan is on screen.
  $("crop-layer").hidden = true;
  $("crop-layer").style.cursor = "";
  $("overlay").toggleAttribute("hidden", !t || t === "crop"); // SVG elements don't reflect .hidden
  // Areas drawn for another tool, or on another view, don't show for this one.
  if (t && t !== "crop") {
    if (fullShown) drawOverlay();
    else $("overlay").innerHTML = "";
  }
  // Center size shows its area with no bar: the slider being moved says what it is.
  $("toolbar").hidden = !t || t === "size";
  // A closing bar keeps its words while it fades.
  if (t && t !== "size") {
    $("tool-text").textContent = TOOL_TEXT[t] ?? "";
    $("tool-cancel").hidden = t === "show";
    $("tool-done").hidden = t === "area" || t === "base";
    $("tool-done").title = t === "crop" ? "Enter" : "Esc";
    $("tool-done").setAttribute("aria-keyshortcuts", t === "crop" ? "Enter" : "Escape");
  }
  // The bar's words, the legend among them, are in place before the image makes room for them.
  showLegend();
  placeStage();
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
const rectInScan = (v, a) => rectFromPoints(viewToScan([v.x0, v.y0], a), viewToScan([v.x1, v.y1], a));
function svgRect(r, cls) {
  const v = rectInView(r, adj());
  return `<rect class="${cls}" x="${v.x0}" y="${v.y0}" width="${v.x1 - v.x0}" height="${v.y1 - v.y0}"/>`;
}
// What the analysis ignores as film holder, while Show areas is on. A custom area is measured as drawn.
const holderKey = () => [...current.settings.estimate.base_rgb, adj().straighten].join();
const holderKnown = () => current.holder?.key === holderKey();
// A film base that's only a guess has no area worth showing: the scan showed no clear film.
// Nor has one taken from the roll: its value didn't come from where this frame's was found.
const baseKnown = () => !!hints().base || (current.baseFound && !hints().base_rgb);
function holderAreas() {
  return tool === "show" && !hints().analysis && holderKnown() ? current.holder.areas : [];
}
// Show areas names what it draws, each followed by a dot in its color, as the Setup labels are.
// The dots pace the list, so it needs no commas.
function showLegend() {
  if (tool !== "show") return;
  // Until the holder is known, keep it named so the text doesn't change under the reader.
  const holder = !hints().analysis && !(holderKnown() && !current.holder.areas.length);
  const items = new Map([...(baseKnown() ? [["film base", "base"]] : []), ["measured area", "area"]]);
  if (adj().crop) items.set("final crop", "crop");
  if (holder) items.set("detected film holder", "holder");
  const list = LIST.formatToParts([...items.keys()]).flatMap((p) => {
    if (p.type !== "element") return [p.value.replace(/,\s*/g, " ")];
    const dot = document.createElement("span");
    dot.className = `dot ${items.get(p.value)}`;
    return [p.value, dot];
  });
  $("tool-text").replaceChildren("Showing the ", ...list, holder ? " auto-ignored in measurement" : "");
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
    const { areas } = await ask("/api/holder", { file, base_rgb, straighten: adj().straighten });
    if (current?.file === file) current.holder = { key, areas };
  } catch (e) {
    if (current?.file === file) couldNot("find the film holder", e);
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
  if (tool === "size") { $("overlay").innerHTML = svgRect(centerArea(), "area"); return; }
  // A centered area shows at the size it's set to, measured yet or not.
  const area = hints().analysis ? e.area : centerArea();
  // The dashed measured area goes over the crop, so both show when they're the same rectangle.
  let s = holderShape() + (adj().crop ? svgRect(adj().crop, "crop") : "") + svgRect(area, "area") + (baseKnown() ? svgRect(e.base_area, "base") : "");
  if (drag) s += `<rect class="drag ${tool}" x="${drag.x0}" y="${drag.y0}" width="${drag.x1 - drag.x0}" height="${drag.y1 - drag.y0}"/>`;
  $("overlay").innerHTML = s;
  showLegend();
}

let dragStart = null;  // where an area is being drawn from, with the pointer that draws it
// The least an area spans of the view, each way. A drag along a narrow film edge is a line, so a
// film base gets this width around it; one that's short both ways is a click, and draws nothing.
// A measured area thin either way is a slip: colors can't be measured on a line.
const AREA_MIN = 0.005;
function drawnArea(p, q) {
  const r = rectFromPoints(p, q);
  const thin = (low, high) => high - low < AREA_MIN;
  if (tool === "base" ? thin(r.x0, r.x1) && thin(r.y0, r.y1) : thin(r.x0, r.x1) || thin(r.y0, r.y1)) return null;
  const widened = (low, high) => {
    if (!thin(low, high)) return [low, high];
    const from = clamp((low + high - AREA_MIN) / 2, 0, 1 - AREA_MIN);
    return [from, from + AREA_MIN];
  };
  [r.x0, r.x1] = widened(r.x0, r.x1);
  [r.y0, r.y1] = widened(r.y0, r.y1);
  return r;
}
const eventPoint = (ev) => {
  const b = $("view").getBoundingClientRect();
  return [clamp((ev.clientX - b.left) / b.width, 0, 1), clamp((ev.clientY - b.top) / b.height, 0, 1)];
};
$("overlay").addEventListener("pointerdown", (ev) => {
  if (!["area", "base"].includes(tool) || ev.button !== 0 || dragStart || touches.size > 1) return;
  dragStart = { pointer: ev.pointerId, at: eventPoint(ev) };
  $("overlay").setPointerCapture(ev.pointerId);
});
// A drag that ends without its pointer coming up here, as when the tool closes under it, draws
// nothing, and leaves the next one free to start.
$("overlay").addEventListener("lostpointercapture", (ev) => { if (dragStart?.pointer === ev.pointerId) dragStart = null; });
$("overlay").addEventListener("pointermove", (ev) => {
  if (dragStart?.pointer === ev.pointerId) drawOverlay(drawnArea(dragStart.at, eventPoint(ev)));
});
// A drag the system takes over, as for a scroll, draws nothing.
$("overlay").addEventListener("pointercancel", (ev) => {
  if (dragStart?.pointer !== ev.pointerId) return;
  dragStart = null;
  drawOverlay();
});
$("overlay").addEventListener("pointerup", (ev) => {
  if (dragStart?.pointer !== ev.pointerId) return;
  const view = drawnArea(dragStart.at, eventPoint(ev));
  dragStart = null;
  if (!view) { drawOverlay(); return; }
  const scan = rectInScan(view, adj());
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
function cropView() {
  return rectInView(cropDraft, adj());
}
function setCropView(v) {
  const a = adj();
  cropDraft = rectInScan(v, a);
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
  if (tool !== "crop" || ev.button !== 0 || cropDrag || touches.size > 1) return;
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
    const full = !!tool, negative = negativeNow(), kind = negative ? "negative" : "positive";
    const key = JSON.stringify([file, full, negative, current.settings]);
    const last = shots[kind];
    let shot = last;
    if (last?.key !== key) {
      const job = { file, settings: structuredClone(current.settings), full, negative };
      const res = await api("/api/preview", job);
      const clipped = Number(res.headers.get("X-Clipped") || 0);
      scanScale = Number(res.headers.get("X-Scale")) || scanScale;
      shot = { key, url: URL.createObjectURL(await res.blob()), clipped, job };
      if (file !== current?.file) { URL.revokeObjectURL(shot.url); return; }
      shots[kind] = shot;
    }
    const img = $("photo");
    // Not img.decode(): it can stay pending for as long as the page is hidden.
    if (img.src !== shot.url) await new Promise((done) => { img.onload = img.onerror = done; img.src = shot.url; });
    if (last && last !== shot) URL.revokeObjectURL(last.url);
    if (file !== current?.file) return;
    fullShown = full;
    shownShot = shot;
    // Areas drawn on the entire scan don't fit a picture that isn't it.
    if (!full && tool && tool !== "crop") $("overlay").innerHTML = "";
    // The sharper part made for this picture shows with it. One made for another doesn't.
    showDetail();
    $("view").hidden = false;
    $("message").hidden = true;
    fit();
    if (tool === "show") {
      await loadHolder();
      if (file !== current?.file) return;
    }
    // A preview requested before a tool opened isn't the entire scan. Another follows it.
    if (full) drawOverlay();
    // The histogram stays the picture's while its negative shows.
    if (!negative) histogram(img, shot.clipped);
  } catch (e) {
    if (file === current?.file) couldNot("update the preview", e);
  } finally {
    rendering = false;
    if (renderAgain) { renderAgain = false; refresh(); }
  }
}

// The stage's padding is the room around the image, which the stylesheet sets: a margin, room for
// the crop frame's handles, and room above for an open tool's bar, which this measures. The image
// eases into what's left, so the bar never covers it. Messages pass over the image and take none.
let barRoom = "";
function placeStage() {
  const room = tool && tool !== "size" ? `${$("floats").offsetTop + $("toolbar").offsetHeight}px` : "0px";
  if (room !== barRoom) $("stage").style.setProperty("--bar-room", barRoom = room);
}
new ResizeObserver(placeStage).observe($("toolbar"));
// While the padding eases, the stage's content box changes, so this follows it every frame.
function fit() {
  const img = $("photo");
  if (!img.naturalWidth) return;
  const box = stageBox();
  const scale = Math.max(0, Math.min(box.width / img.naturalWidth, box.height / img.naturalHeight));
  fitSize = { width: Math.max(1, img.naturalWidth * scale), height: Math.max(1, img.naturalHeight * scale) };
  // The zoom stays while the image keeps its size, from frame to frame too, for checking a roll
  // at one place. Another size is another framing, which starts in full view.
  const size = `${img.naturalWidth}×${img.naturalHeight}`;
  if (size !== zoomedSize) {
    zoomedSize = size;
    Object.assign(zoom, ZOOM_OUT);
  }
  placeView(box);
}
new ResizeObserver(fit).observe($("stage"));

// ---------- Zoom ----------
// The image fits the room between the panels, and zooms from there up to twice the scan's own
// pixels: a pinch or Control and the wheel zoom around where they point, and a double click or
// tap goes to 100%, one scan pixel to one of the screen's, and back. Zoomed in, two fingers on a
// touch screen or trackpad move the image, or a drag with Space held, so one finger or the mouse
// stays free for a tool that draws. With no such tool open, one finger moves the image too. The
// stage clips the image, and its bars and messages stay over it.
// Zooming is only a way of looking: it changes no setting, so nothing is measured or saved for it.
const ZOOM_OUT = { z: 1, x: 0, y: 0 };
// How far the image is zoomed from its fit, and how far its center is from the room's, in px.
const zoom = { ...ZOOM_OUT };
let fitSize = { width: 1, height: 1 };
let zoomedSize = "";  // the size of the image the zoom was set on
let scanScale = 4;    // scan pixels to one of the preview's, each way
const ZOOM_SNAP = 1.03;      // closer to the fit than this is the fit
const ZOOM_WHEEL = 0.01;     // zoom per wheel step, as an exponent
const ZOOM_MIN_GAIN = 1.15;  // a 100% view this close to the fit isn't worth a double click

// The room the image fits: the stage inside its padding, and where its center is on the page.
function stageBox() {
  const stage = $("stage"), s = getComputedStyle(stage), r = stage.getBoundingClientRect();
  const left = parseFloat(s.paddingLeft), top = parseFloat(s.paddingTop);
  const width = stage.clientWidth - left - parseFloat(s.paddingRight);
  const height = stage.clientHeight - top - parseFloat(s.paddingBottom);
  return { width, height, x: r.left + left + width / 2, y: r.top + top + height / 2 };
}
// The zoom that shows one scan pixel on one of the screen's, and the most there is.
const zoomFull = () => $("photo").naturalWidth * scanScale / devicePixelRatio / fitSize.width;
const zoomMax = () => Math.max(1, 2 * zoomFull());
// Sizes and moves the image for the zoom, kept within what there is to see: the image never
// leaves room beside it that it could fill.
function placeView(box = stageBox()) {
  const view = $("view");
  zoom.z = clamp(zoom.z, 1, zoomMax());
  const width = fitSize.width * zoom.z, height = fitSize.height * zoom.z;
  const reach = (size, room) => Math.max(0, (size - room) / 2);
  zoom.x = clamp(zoom.x, -reach(width, box.width), reach(width, box.width));
  zoom.y = clamp(zoom.y, -reach(height, box.height), reach(height, box.height));
  view.style.width = `${width}px`;
  view.style.height = `${height}px`;
  view.style.transform = `translate(${zoom.x}px, ${zoom.y}px)`;
  if (zoom.z > 1) wantDetail();
}
// Zooms to `z`, keeping the point of the image at `at` on the page where it is.
function zoomTo(z, at, eased = false) {
  const box = stageBox(), view = $("view");
  z = clamp(z, 1, zoomMax());
  if (z < ZOOM_SNAP) z = 1;
  const place = () => ({ width: view.style.width, height: view.style.height, transform: view.style.transform });
  const from = place(), ratio = z / zoom.z, x = at.x - box.x, y = at.y - box.y;
  Object.assign(zoom, { z, x: x - (x - zoom.x) * ratio, y: y - (y - zoom.y) * ratio });
  placeView(box);
  const to = place();
  if (eased) move(view, "zoom", { width: [from.width, to.width], height: [from.height, to.height], transform: [from.transform, to.transform] }, motion("--slide"), { after: wantDetail });
  else endMotion(view, "zoom");
}
function zoomOut() {
  endMotion($("view"), "zoom");
  if (zoom.z === 1) return;
  Object.assign(zoom, ZOOM_OUT);
  placeView();
}
onDouble($("stage"), (e) => {
  if (!current || !e.target.closest("#view")) return;
  const full = zoomFull();
  zoomTo(zoom.z > 1 ? 1 : full > ZOOM_MIN_GAIN ? full : zoomMax(), { x: e.clientX, y: e.clientY }, true);
});
// A drawing or a crop drag under way gives way to a move of the image.
function cancelDrags() {
  if (dragStart) {
    dragStart = null;
    drawOverlay();
  }
  if (cropDrag) {
    setCropView(cropDrag.from);
    cropDrag = null;
    $("crop-layer").style.cursor = "";
  }
}

// Two fingers on the stage pinch and move the image, wherever on it they land: the stage hears
// of them before the tool under them does, and ends what one finger had begun there. A tool
// starts nothing with a second finger.
const touches = new Map();  // the fingers on the stage, by pointer
let pinch = null;           // the zoom when two fingers came down, where between them, and how far apart
let fingerPan = null;       // one finger moving the zoomed image: its pointer, where it began, and where the image was
// The tools that take one finger's drag for themselves.
const toolDraws = () => ["crop", "area", "base"].includes(tool);
const fingers = () => {
  const [a, b] = [...touches.values()];
  return { at: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, apart: Math.hypot(a.x - b.x, a.y - b.y) };
};
function touchesChanged() {
  if (touches.size === 2) {
    cancelDrags();
    pinch = { ...zoom, ...fingers() };
  } else {
    pinch = null;
    if (zoom.z < ZOOM_SNAP) zoomOut();
  }
  // The one finger down, or the one left of a pinch, moves the zoomed image from where it is now,
  // unless it came down on a bar or a message.
  const [[pointer, only] = []] = touches.size === 1 ? [...touches] : [];
  fingerPan = only?.onImage && zoom.z > 1 && !toolDraws() ? { pointer, x: only.x, y: only.y, from: { x: zoom.x, y: zoom.y } } : null;
}
$("stage").addEventListener("pointerdown", (e) => {
  if (e.pointerType !== "touch") return;
  touches.set(e.pointerId, { x: e.clientX, y: e.clientY, onImage: !e.target.closest(".float") });
  touchesChanged();
}, true);
$("stage").addEventListener("pointermove", (e) => {
  const touch = touches.get(e.pointerId);
  if (!touch) return;
  Object.assign(touch, { x: e.clientX, y: e.clientY });
  if (fingerPan?.pointer === e.pointerId && current) {
    zoom.x = fingerPan.from.x + e.clientX - fingerPan.x;
    zoom.y = fingerPan.from.y + e.clientY - fingerPan.y;
    endMotion($("view"), "zoom");
    placeView();
    return;
  }
  if (!pinch || !current) return;
  const now = fingers(), box = stageBox();
  const z = clamp(pinch.z * now.apart / Math.max(1, pinch.apart), 1, zoomMax()), ratio = z / pinch.z;
  // The point between the fingers stays between them as they move and spread.
  Object.assign(zoom, { z, x: now.at.x - box.x - (pinch.at.x - box.x - pinch.x) * ratio, y: now.at.y - box.y - (pinch.at.y - box.y - pinch.y) * ratio });
  endMotion($("view"), "zoom");
  placeView(box);
}, true);
for (const end of ["pointerup", "pointercancel"]) {
  // On the window: a finger that began on the stage can come up anywhere.
  addEventListener(end, (e) => { if (touches.delete(e.pointerId)) touchesChanged(); }, true);
}
// Fingers the page stops hearing of, as when the app goes to the background, are no longer down.
for (const type of ["blur", "visibilitychange"]) {
  addEventListener(type, () => {
    if (!touches.size) return;
    touches.clear();
    touchesChanged();
  });
}
// A trackpad's pinch comes as a wheel turn with Control held in some browsers and as a gesture in
// others, and its two-finger move as a plain wheel turn.
$("stage").addEventListener("wheel", (e) => {
  if (!current || $("view").hidden || e.target.closest(".float")) return;
  const lines = e.deltaMode === 1 ? 16 : 1;
  if (e.ctrlKey) {
    e.preventDefault();
    zoomTo(zoom.z * Math.exp(-e.deltaY * lines * ZOOM_WHEEL), { x: e.clientX, y: e.clientY });
  } else if (zoom.z > 1) {
    e.preventDefault();
    zoom.x -= e.deltaX * lines;
    zoom.y -= e.deltaY * lines;
    endMotion($("view"), "zoom");
    placeView();
  }
}, { passive: false });
let gestureFrom = 0;  // the zoom a trackpad's pinch gesture began at; fingers on a screen are followed themselves
$("stage").addEventListener("gesturestart", () => { gestureFrom = touches.size ? 0 : zoom.z; });
$("stage").addEventListener("gesturechange", (e) => { if (gestureFrom && !touches.size && current) zoomTo(gestureFrom * e.scale, { x: e.clientX, y: e.clientY }); });
$("stage").addEventListener("gestureend", () => { gestureFrom = 0; });

// Space held with the pointer over the image turns a drag into a move of it, before any tool
// hears of the drag. Elsewhere Space keeps its own meaning, as on a button: over a bar or a
// message on the image too, and in a dialog.
let overStage = false, spaceHeld = false;
let panDrag = null;  // { pointer, x, y, from: where the image was }
$("stage").addEventListener("pointerenter", () => { overStage = true; });
$("stage").addEventListener("pointerleave", () => { overStage = false; });
function showPan() {
  if (panDrag) document.body.dataset.pan = "drag";
  else if (spaceHeld) document.body.dataset.pan = "";
  else delete document.body.dataset.pan;
}
document.addEventListener("keydown", (e) => {
  if (e.code !== "Space" || e.target.matches?.("input[type=text]") || document.querySelector("dialog[open], .float:hover")) return;
  if (!(overStage || spaceHeld)) return;
  e.preventDefault();
  spaceHeld = true;
  showPan();
}, true);
function releaseSpace() {
  spaceHeld = false;
  showPan();
}
document.addEventListener("keyup", (e) => { if (e.code === "Space") releaseSpace(); }, true);
addEventListener("blur", releaseSpace);
$("stage").addEventListener("pointerdown", (e) => {
  if (!spaceHeld || e.button !== 0 || e.pointerType === "touch" || panDrag) return;
  e.preventDefault();
  e.stopPropagation();
  panDrag = { pointer: e.pointerId, x: e.clientX, y: e.clientY, from: { x: zoom.x, y: zoom.y } };
  $("stage").setPointerCapture(e.pointerId);
  showPan();
}, true);
$("stage").addEventListener("pointermove", (e) => {
  if (panDrag?.pointer !== e.pointerId) return;
  zoom.x = panDrag.from.x + e.clientX - panDrag.x;
  zoom.y = panDrag.from.y + e.clientY - panDrag.y;
  endMotion($("view"), "zoom");
  placeView();
});
for (const end of ["pointerup", "pointercancel", "lostpointercapture"]) {
  $("stage").addEventListener(end, (e) => {
    if (panDrag?.pointer !== e.pointerId) return;
    panDrag = null;
    showPan();
  });
}

// The preview has a quarter of the scan's pixels each way, which is all a fitted image shows and
// is quick to render while a slider moves. Zoomed past those pixels, the part in view comes from
// the full scan and lies over the preview, which shows meanwhile. It's asked for once the view
// and the settings have rested a moment, so a slider or a pinch never waits for it, and with some
// room around it, so a short move needs no new one.
const DETAIL_PAUSE_MS = 200;
const DETAIL_GAIN = 1.2;      // screen pixels to one of the preview's, from which the scan shows more
const DETAIL_MARGIN = 0.15;   // room around the part in view, as a share of it
const DETAIL_MAX_SIDE = 3200;
// The last sharper part of each kind, as { key, part, gain, url }: the preview it was made for,
// the part it covers and the gain it's sharp to. Like the previews, one each for the picture and
// its negative, so going between them stays sharp.
const details = { positive: null, negative: null };
let detailTimer = 0, detailLoading = false, detailAgain = false;
const kindOf = (shot) => (shot.job.negative ? "negative" : "positive");
// The sharper part made for the preview on screen, if there is one.
const detailNow = () => {
  const has = shownShot && !$("view").hidden && details[kindOf(shownShot)];
  return has && has.key === shownShot.key ? has : null;
};
function wantDetail() {
  clearTimeout(detailTimer);
  detailTimer = setTimeout(loadDetail, DETAIL_PAUSE_MS);
}
// Lays the sharper part made for the preview on screen over it, or hides the one that isn't.
function showDetail() {
  const detail = $("detail"), has = detailNow();
  detail.hidden = !has;
  if (!has) return;
  for (const [side, v] of [["left", has.part.x0], ["top", has.part.y0], ["width", has.part.x1 - has.part.x0], ["height", has.part.y1 - has.part.y0]]) detail.style[side] = `${v * 100}%`;
  if (detail.src !== has.url) detail.src = has.url;
}
async function loadDetail() {
  if (detailLoading) { detailAgain = true; return; }
  const shot = shownShot, view = $("view").getBoundingClientRect(), stage = $("stage").getBoundingClientRect();
  const gain = view.width * devicePixelRatio / $("photo").naturalWidth;
  // Only a zoomed image gets one: a fitted one is what sliders work on, and they mustn't wait for
  // the full scan. A running save has the engine, and the memory a full scan takes.
  if (!current || !shot || $("view").hidden || zoom.z === 1 || gain < DETAIL_GAIN || batch || motionTag($("view"), "zoom") != null) return;
  const seen = {
    x0: clamp((stage.left - view.left) / view.width, 0, 1), y0: clamp((stage.top - view.top) / view.height, 0, 1),
    x1: clamp((stage.right - view.left) / view.width, 0, 1), y1: clamp((stage.bottom - view.top) / view.height, 0, 1),
  };
  if (seen.x1 <= seen.x0 || seen.y1 <= seen.y0) return;
  const has = detailNow();
  const covered = has && has.part.x0 <= seen.x0 && has.part.y0 <= seen.y0 && has.part.x1 >= seen.x1 && has.part.y1 >= seen.y1;
  // Sharp enough for this zoom, or as sharp as the scan gets.
  if (covered && has.gain >= Math.min(gain, scanScale) / DETAIL_GAIN) return;
  const roomX = (seen.x1 - seen.x0) * DETAIL_MARGIN, roomY = (seen.y1 - seen.y0) * DETAIL_MARGIN;
  const part = { x0: Math.max(0, seen.x0 - roomX), y0: Math.max(0, seen.y0 - roomY), x1: Math.min(1, seen.x1 + roomX), y1: Math.min(1, seen.y1 + roomY) };
  const side = Math.max((part.x1 - part.x0) * view.width, (part.y1 - part.y0) * view.height) * devicePixelRatio;
  const pixels = Math.min(DETAIL_MAX_SIDE, Math.ceil(side));
  detailLoading = true;
  try {
    const res = await api("/api/detail", { ...shot.job, part, pixels });
    const url = URL.createObjectURL(await res.blob());
    // Loaded apart from the page, so it shows whole, and a picture that changes meanwhile can't
    // leave this waiting.
    const loaded = new Image();
    await new Promise((done) => { loaded.onload = loaded.onerror = done; loaded.src = url; });
    const kind = kindOf(shot), last = details[kind];
    details[kind] = { key: shot.key, part, gain: gain * pixels / side, url };
    showDetail();
    if (last) URL.revokeObjectURL(last.url);
  } catch {
    // The preview stays, as sharp as it is.
  } finally {
    detailLoading = false;
    if (detailAgain) { detailAgain = false; loadDetail(); }
  }
}

const histCanvas = document.createElement("canvas");
// The share of the preview's pixels at one end that's worth a warning, and the share of the DNG's
// that's worth a line of its own.
const CLIP_SHOWN = 0.005, CLIP_STORED = 0.0005;
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
  const lowOver = low / n > CLIP_SHOWN, highOver = high / n > CLIP_SHOWN, storedOver = storedClipped >= CLIP_STORED;
  $("clip-low").classList.toggle("over", lowOver);
  $("clip-high").classList.toggle("over", highOver);
  $("clip-high").title = highOver && !storedOver ? "The DNG keeps these. Lower Whites or use Flat to see them." : "";
  // "Clipped" above is only the preview's and the DNG keeps it; these pixels are gone for good.
  // The words go in first, so the box grows to their height.
  $("stored").textContent = `${(storedClipped * 100).toFixed(2)}% of highlights beyond DNG range`;
  $("stored").title = "Brighter than the DNG can store. Lower Exposure to keep them.";
  showHistRows(lowOver || highOver, storedOver);
}

ask("/api/version").then(({ version, commit }) => {
  const label = commit ? `${version} · ${commit}` : version;
  $("about-open").textContent = label;
  $("about-title").textContent = `Reversal ${label}`;
}).catch(() => {});
$("about-open").addEventListener("click", () => { $("about").showModal(); showNotices(); });
// An app carries the notices of what's built into it, and this is where they can be read.
// Each is fetched when it's opened.
let noticesShown = false;
async function showNotices() {
  if (noticesShown) return;
  noticesShown = true;
  try {
    const { notices } = await ask("/api/notices");
    $("about-notices").replaceChildren(...notices.map((name) => {
      const notice = document.createElement("details"), title = document.createElement("summary"), text = document.createElement("pre");
      title.textContent = name.replace(/\.txt$/, "");
      notice.append(title, text);
      notice.addEventListener("toggle", () => {
        if (!text.textContent) api(`/api/notice?name=${encodeURIComponent(name)}`).then((r) => r.text()).then((t) => { text.textContent = t; }).catch(() => {});
      });
      return notice;
    }));
  } catch {
    noticesShown = false;
  }
}
$("about-close").addEventListener("click", () => $("about").close());
$("about-details").addEventListener("toggle", () => {
  if ($("about-license").textContent) return;
  api("/api/license").then((r) => r.text()).then((text) => { $("about-license").textContent = text; }).catch(() => {});
});

$("choose-folder").disabled = !shell;
updateSaveButton();
setBusy(false);
showHistRows(false, false, { now: true });
for (const id of ["choose-folder", "start-open"]) $(id).addEventListener("click", () => toShell("chooseFolder"));

loadFiles().then(() => {
  // A folder opens on its first frame: an empty stage only asks for a tap that's always the same.
  if (!current && !opening && frames.length) open(frames[0].name);
  const left = unfinished.length;
  offerRest(`Saving stopped with ${plural(left, "frame")} left`, left > 1 ? "Save them" : "Save it");
}).catch((e) => { $("message").textContent = `Could not list files: ${e.message}`; });
// Reloaded while saving: pick the progress back up.
ask("/api/export/status").then((s) => { if (s.running) { batch = s; updateSaveButton(); followBatch(); } }).catch(() => {});
addEventListener("pagehide", () => { rememberSettings(true).catch(() => {}); });
