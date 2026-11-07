const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const view = document.getElementById("view");
const layer = document.getElementById("layer");
const toastEl = document.getElementById("toast");

// The rows of "Send detected files somewhere else": what a file was detected as.
const OVERRIDE_KINDS = [
  { detected: "bambu-studio", label: "Bambu project" },
  { detected: "snapmaker-orca", label: "Snapmaker project" },
  { detected: "prusa-slicer", label: "Prusa project" },
  { detected: "orca-slicer", label: "OrcaSlicer file" },
];

// Everything the window remembers besides what Rust sends. Rust owns the files,
// failures, config and default-app status in `data`.
//
// One sheet shows at a time. `sheet` is the mounted one: null, or
// { kind: "question" | "failure", key }, and for a failure sheet the `files` it
// lists, which is what its buttons act on. A question outranks failures.
// `deferred` holds the keys of the question groups the person pressed Later on;
// a group not in it opens its sheet. `leaving` is
// set from the moment a sheet starts to exit until the action that closed it has
// finished, and nothing new mounts in between.
const ui = {
  data: null,
  icons: null,
  sheet: null,
  leaving: false,
  deferred: new Set(),
  overridesOpen: false,
  stepsOpen: false,
  problem: null,
};

// Children may be nested lists, and `false` or `null` for "not here".
const compact = (children) => children.flat().filter((child) => child != null && child !== false);

function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (value == null || value === false) continue;
    if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
    else node.setAttribute(name, value === true ? "" : value);
  }
  node.append(...compact(children));
  return node;
}

const files = (count) => `${count} ${count === 1 ? "file" : "files"}`;
const baseName = (path) => path.split(/[\\/]/).pop();
const slicerName = (id) => ui.data.installed.find((slicer) => slicer.id === id)?.name ?? id;
const unique = (items) => [...new Set(items)];

/* State from Rust */

let lastState = "";
let refreshing = null;
let refreshAgain = false;

// Callers that await this see the newest state, even when more refreshes were
// asked for while one was in flight.
function refresh() {
  if (refreshing) {
    refreshAgain = true;
    return refreshing;
  }
  refreshing = (async () => {
    do {
      refreshAgain = false;
      await load();
    } while (refreshAgain);
    refreshing = null;
  })();
  return refreshing;
}

async function load() {
  let state;
  try {
    state = await invoke("get_state");
  } catch (error) {
    ui.problem = String(error);
    if (ui.data) render();
    return;
  }
  const text = JSON.stringify(state);
  if (text === lastState) return;
  const slicersChanged =
    !ui.data || JSON.stringify(state.installed) !== JSON.stringify(ui.data.installed);
  lastState = text;
  ui.data = state;
  if (slicersChanged) loadIcons();
  render();
}

async function loadIcons() {
  try {
    ui.icons = await invoke("slicer_icons");
  } catch {
    ui.icons = {};
  }
  render();
}

// Run a command. A failure becomes the problem card and a success clears it.
async function command(name, args) {
  try {
    await invoke(name, args);
    if (ui.problem) {
      ui.problem = null;
      render();
    }
    return true;
  } catch (error) {
    ui.problem = String(error);
    render();
    return false;
  }
}

/* Main view */

function render() {
  if (!ui.data) return;
  const waiting = new Set(ui.data.pending.map(questionKey));
  for (const key of ui.deferred) if (!waiting.has(key)) ui.deferred.delete(key);
  renderView();
  reconcileSheet();
}

function renderView() {
  const focused = document.activeElement;
  const key = view.contains(focused) ? focused.dataset.key : null;
  view.replaceChildren(...compact(buildView(ui.data)));
  if (key) view.querySelector(`[data-key="${CSS.escape(key)}"]`)?.focus({ preventScroll: true });
}

function buildView(data) {
  const hasSlicers = data.installed.length > 0;
  return [
    h(
      "header",
      { class: "hdr" },
      h("img", { src: "router.svg", alt: "" }),
      h("h1", { class: "word" }, "Slicer Router"),
    ),
    ui.problem && problemCard(ui.problem),
    !hasSlicers && emptyCard(),
    waitingLine(data),
    defaultAppCard(data),
    hasSlicers && [
      h("h2", { class: "headline", id: "default-label" }, "Files with no slicer info", h("br"), "open in"),
      slicerTiles(data),
      overrides(data),
    ],
    defaultAppStatus(data),
  ];
}

function problemCard(message) {
  return h(
    "div",
    { class: "card err", role: "alert" },
    h("div", { class: "card-text" }, message),
    h(
      "button",
      {
        class: "link press",
        type: "button",
        "data-key": "problem-dismiss",
        onclick: () => {
          ui.problem = null;
          render();
        },
      },
      "Dismiss",
    ),
  );
}

function emptyCard() {
  return h(
    "div",
    { class: "card warn" },
    h("div", { class: "card-title" }, "No supported slicer found"),
    h(
      "div",
      { class: "card-text" },
      "Install Bambu Studio, OrcaSlicer, Snapmaker Orca or PrusaSlicer, then reopen Slicer Router.",
    ),
  );
}

function waitingLine(data) {
  const deferred = data.pending.filter((group) => ui.deferred.has(questionKey(group)));
  if (deferred.length === 0) return null;
  const count = deferred.reduce((total, group) => total + group.files.length, 0);
  return h(
    "div",
    { class: "card warn waitline" },
    h("span", {}, `${files(count)} still ${count === 1 ? "needs" : "need"} a slicer`),
    h("button", { class: "btn press", type: "button", "data-key": "reopen", onclick: reopenQuestion }, "Choose"),
  );
}

function slicerMark(slicer) {
  if (ui.icons === null) return h("span", { class: "mark" });
  const icon = ui.icons[slicer.id];
  return icon
    ? h("img", { class: "mark", src: icon, alt: "" })
    : h("span", { class: "mark letter", "aria-hidden": "true" }, slicer.name[0]);
}

function tile(slicer, attrs, onclick) {
  return h(
    "button",
    { class: "tile press", type: "button", ...attrs, onclick },
    slicerMark(slicer),
    h("span", { class: "tile-name" }, slicer.name),
  );
}

// The default slicer for files that carry no slicer info. A radio group: one
// tab stop, and the arrow keys move between the tiles.
function slicerTiles(data) {
  const selected = data.installed.some((slicer) => slicer.id === data.default_slicer)
    ? data.default_slicer
    : data.installed[0].id;
  const group = h(
    "div",
    { class: "tiles", role: "radiogroup", "aria-labelledby": "default-label" },
    ...data.installed.map((slicer) =>
      tile(
        slicer,
        {
          role: "radio",
          "aria-checked": String(slicer.id === data.default_slicer),
          tabindex: slicer.id === selected ? "0" : "-1",
          "data-key": `slicer:${slicer.id}`,
        },
        () => chooseDefault(slicer),
      ),
    ),
  );
  group.addEventListener("keydown", (event) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
    if (!step) return;
    event.preventDefault();
    const tiles = [...group.querySelectorAll(".tile")];
    const next = tiles[(tiles.indexOf(document.activeElement) + step + tiles.length) % tiles.length];
    next.focus();
  });
  return group;
}

// What a file does when its own slicer is not installed and no override applies,
// the way route() in the core crate decides: the default slicer, else a lone
// installed slicer, else the person is asked.
function unplacedLabel(data) {
  const fallback = data.installed.find((slicer) => slicer.id === data.default_slicer);
  if (fallback) return `Default (${fallback.name})`;
  if (data.installed.length === 1) return `Only installed (${data.installed[0].name})`;
  return "Ask me";
}

function overrides(data) {
  const installed = new Set(data.installed.map((slicer) => slicer.id));
  const rows = OVERRIDE_KINDS.map((kind) => {
    const target = data.overrides[kind.detected];
    let current = "";
    if (installed.has(target)) current = target;
    else if (installed.has(kind.detected)) current = kind.detected;
    const id = `override-${kind.detected}`;
    return h(
      "div",
      { class: "orow" },
      h("label", { for: id }, kind.label),
      h(
        "select",
        { id, "data-key": id, onchange: (event) => setOverride(kind, event.target.value || null) },
        !installed.has(kind.detected) && h("option", { value: "", selected: current === "" }, unplacedLabel(data)),
        ...data.installed.map((slicer) =>
          h(
            "option",
            { value: slicer.id, selected: slicer.id === current },
            slicer.id === kind.detected ? `${slicer.name} (detected)` : slicer.name,
          ),
        ),
      ),
    );
  });
  const details = h(
    "details",
    { class: "adv", open: ui.overridesOpen },
    h("summary", {}, "Send detected files somewhere else"),
    h("div", { class: "ovr" }, ...rows),
    h("p", { class: "fine" }, "Most people never change these."),
  );
  details.addEventListener("toggle", () => {
    ui.overridesOpen = details.open;
  });
  return details;
}

function defaultAppCard(data) {
  const app = data.default_app;
  if (!app || app.is_default) return null;
  const title = h("div", { class: "card-title" }, `.3mf files still open in ${app.current_name}`);
  if (data.platform !== "windows") {
    return h(
      "div",
      { class: "card warn" },
      title,
      h("div", { class: "card-text" }, "Slicer Router can switch it for you."),
      h(
        "button",
        { class: "btn bone press", type: "button", "data-key": "make-default", onclick: makeDefault },
        "Make Slicer Router the default",
      ),
    );
  }
  return h(
    "div",
    { class: "card warn" },
    title,
    h("div", { class: "card-text" }, "Windows makes you pick this yourself. We'll select a .3mf for you."),
    h(
      "div",
      { class: "actions" },
      h(
        "button",
        { class: "btn bone press", type: "button", "data-key": "reveal", onclick: revealExample },
        "Reveal a .3mf in Explorer",
      ),
      h(
        "button",
        {
          class: "btn press",
          type: "button",
          "data-key": "steps",
          "aria-expanded": String(ui.stepsOpen),
          "aria-controls": "windows-steps",
          onclick: () => {
            ui.stepsOpen = !ui.stepsOpen;
            render();
          },
        },
        ui.stepsOpen ? "Hide steps" : "Show steps",
      ),
    ),
    ui.stepsOpen && windowsSteps(),
  );
}

function windowsSteps() {
  return h(
    "div",
    { id: "windows-steps" },
    h(
      "ol",
      { class: "steps" },
      h("li", {}, "Right-click the selected ", h("code", {}, ".3mf"), " and choose ", h("b", {}, "Open with › Choose another app"), "."),
      h("li", {}, "Pick ", h("b", {}, "Slicer Router"), "."),
      h("li", {}, "Click ", h("b", {}, "Always"), "."),
    ),
    h(
      "p",
      { class: "fine" },
      "Then turn off Associate .3mf files in each slicer's preferences. Otherwise it takes .3mf back the next time it starts.",
    ),
  );
}

function defaultAppStatus(data) {
  if (!data.default_app?.is_default) return null;
  return h("p", { class: "status" }, h("span", { class: "dot" }), "Slicer Router opens .3mf files");
}

/* Actions */

let toastTimer;
function toast(text) {
  toastEl.textContent = text;
  toastEl.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("on"), 2200);
}

async function chooseDefault(slicer) {
  const waiting = ui.data.pending.find((group) => group.detected === null);
  if (!(await command("choose", { detected: null, slicer: slicer.id }))) return;
  await refresh();
  toast(
    waiting
      ? `Opening ${files(waiting.files.length)} in ${slicer.name}`
      : `Saved · no slicer info → ${slicer.name}`,
  );
}

async function setOverride(kind, slicerId) {
  if (!(await command("set_override", { detected: kind.detected, slicer: slicerId }))) return;
  await refresh();
  toast(`Saved · ${kind.label} → ${slicerId ? slicerName(slicerId) : unplacedLabel(ui.data)}`);
}

async function makeDefault() {
  if (!(await command("make_default_app"))) return;
  await refresh();
  toast("Slicer Router now opens .3mf files");
}

async function revealExample() {
  await command("reveal_example_3mf");
  render();
}

/* Sheets */

function questionKey(group) {
  return `question:${group.detected ?? ""}`;
}

function wantedSheet(data) {
  const group = data.pending.find((candidate) => !ui.deferred.has(questionKey(candidate)));
  if (group) return { kind: "question", key: questionKey(group) };
  if (data.failures.length > 0) return { kind: "failure", key: "failure" };
  return null;
}

function reconcileSheet() {
  if (ui.leaving) return;
  const want = wantedSheet(ui.data);
  if (ui.sheet && want && ui.sheet.key === want.key) {
    fillSheet(layer.querySelector(".sheet"), want);
    return;
  }
  if (ui.sheet) {
    ui.leaving = true;
    closeSheet().then(() => {
      ui.leaving = false;
      reconcileSheet();
    });
    return;
  }
  if (want) openSheet(want);
}

function openSheet(want) {
  ui.sheet = want;
  const sheet = h("div", {
    class: `sheet ${want.kind}`,
    role: want.kind === "failure" ? "alertdialog" : "dialog",
    "aria-modal": "true",
    "aria-labelledby": "sheet-title",
    tabindex: "-1",
  });
  fillSheet(sheet, want);
  layer.replaceChildren(h("div", { class: "scrim", onclick: dismissSheet }), sheet);
  view.toggleAttribute("inert", true);
  sheet.focus({ preventScroll: true });
}

function fillSheet(sheet, want) {
  const focused = document.activeElement;
  const key = sheet.contains(focused) ? focused.dataset.key : null;
  if (want.kind === "failure") ui.sheet.files = ui.data.failures.map((failure) => failure.file);
  const { title, body, close } =
    want.kind === "question"
      ? questionSheet(ui.data.pending.find((group) => questionKey(group) === want.key))
      : failureSheet(ui.data);
  sheet.replaceChildren(
    h(
      "div",
      { class: "shead" },
      h("div", { class: "sheet-title", id: "sheet-title" }, title),
      h(
        "button",
        {
          class: "later press",
          type: "button",
          "data-key": "close",
          "aria-keyshortcuts": "Escape",
          onclick: dismissSheet,
        },
        close,
        h("span", { class: "kbd", "aria-hidden": "true" }, "esc"),
      ),
    ),
    ...compact(body),
  );
  if (key) sheet.querySelector(`[data-key="${CSS.escape(key)}"]`)?.focus({ preventScroll: true });
}

// Slide the sheet out. Resolves when it is gone and the window is usable again.
function closeSheet() {
  const sheet = layer.querySelector(".sheet");
  const scrim = layer.querySelector(".scrim");
  sheet.classList.add("out");
  scrim.classList.add("out");
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      layer.replaceChildren();
      view.removeAttribute("inert");
      ui.sheet = null;
      resolve();
    };
    sheet.addEventListener("animationend", finish, { once: true });
    setTimeout(finish, 300);
  });
}

// Close the sheet while `action` runs, then show what comes next. Resolves with
// what `action` returned.
async function leaveSheet(action) {
  ui.leaving = true;
  const closing = closeSheet();
  const result = await action();
  await refresh();
  await closing;
  ui.leaving = false;
  render();
  return result;
}

function dismissSheet() {
  if (ui.leaving || !ui.sheet) return;
  if (ui.sheet.kind === "question") deferQuestion();
  else dismissFailures();
}

// A toast that says files are opening would contradict a failure sheet.
const failureShowing = () => ui.sheet?.kind === "failure";

function focusSelectedTile() {
  if (ui.sheet) return;
  (view.querySelector('.tile[aria-checked="true"]') ?? view.querySelector(".tile"))?.focus({
    preventScroll: true,
  });
}

function questionSheet(group) {
  const count = group.files.length;
  const name = group.detected_name;
  const title = name
    ? `${count} ${name} ${count === 1 ? "file" : "files"}, but ${name} isn't installed`
    : `${files(count)} ${count === 1 ? "has" : "have"} no slicer info`;
  const note = name
    ? `Your pick is remembered for ${name} files.`
    : "Your pick becomes the default for files like these.";
  return {
    title,
    close: "Later",
    body: [
      fileList(group.files.map((file) => ({ file }))),
      h(
        "div",
        { class: "tiles" },
        ...ui.data.installed.map((slicer) =>
          tile(slicer, { "data-key": `pick:${slicer.id}` }, () => answerQuestion(group, slicer.id)),
        ),
      ),
      h("p", { class: "fine" }, note),
    ],
  };
}

function failureSheet(data) {
  const { failures, installed } = data;
  const failedSlicers = new Set(failures.map((failure) => failure.slicer));
  const alternatives = installed.filter((slicer) => !failedSlicers.has(slicer.id));
  const retrySlicers = unique(failures.filter((failure) => failure.retryable).map((failure) => failure.slicer));
  const retryLabel = retrySlicers.length === 1 ? `Try ${slicerName(retrySlicers[0])} again` : "Try again";
  return {
    title: failureTitle(failures),
    close: "Dismiss",
    body: [
      fileList(failures),
      alternatives.length > 0 && [
        h("div", { class: "label" }, "Open in another slicer"),
        h(
          "div",
          { class: "tiles" },
          ...alternatives.map((slicer) =>
            tile(slicer, { "data-key": `open:${slicer.id}` }, () => openElsewhere(slicer.id)),
          ),
        ),
      ],
      installed.length === 0 &&
        h(
          "p",
          { class: "card-text" },
          "Install Bambu Studio, OrcaSlicer, Snapmaker Orca or PrusaSlicer, then open the file again.",
        ),
      retrySlicers.length > 0 &&
        h(
          "div",
          { class: "actions" },
          h("button", { class: "btn press", type: "button", "data-key": "retry", onclick: retryFailures }, retryLabel),
        ),
    ],
  };
}

function failureTitle(failures) {
  const count = failures.length;
  const slicers = unique(failures.map((failure) => failure.slicer));
  if (slicers.length > 1) return `Couldn't open ${files(count)}`;
  const [{ slicer_name: name }] = failures;
  return name ? `${name} couldn't open ${files(count)}` : `No slicer to open ${files(count)}`;
}

// Up to three base names, each with its reason when it failed, and a count of the rest.
function fileList(entries) {
  const shown = entries.slice(0, 3);
  const more = entries.length - shown.length;
  return h(
    "ul",
    { class: "files" },
    ...shown.map(({ file, reason }) =>
      h("li", { class: "path", title: file }, baseName(file), reason && [" · ", h("span", { class: "reason" }, reason)]),
    ),
    more > 0 && h("li", { class: "path" }, `+ ${more} more`),
  );
}

async function answerQuestion(group, slicerId) {
  if (ui.leaving) return;
  const count = group.files.length;
  const opened = await leaveSheet(() => command("choose", { detected: group.detected, slicer: slicerId }));
  if (!opened) return;
  if (!failureShowing()) toast(`Opening ${files(count)} in ${slicerName(slicerId)}`);
  landOnTile(slicerId, group.detected === null);
}

// Focus the tile the person chose. A pick for files with no slicer info becomes
// the default, so that tile pulses once; a pick for a missing slicer is an
// override, which no tile shows.
function landOnTile(slicerId, pulse) {
  const landed = view.querySelector(`[data-key="slicer:${CSS.escape(slicerId)}"]`);
  if (!landed) return;
  if (!ui.sheet) landed.focus({ preventScroll: true });
  if (pulse) landed.classList.add("landed");
}

async function deferQuestion() {
  const { key } = ui.sheet;
  await leaveSheet(() => {
    ui.deferred.add(key);
  });
  if (!ui.sheet) view.querySelector('[data-key="reopen"]')?.focus({ preventScroll: true });
}

function reopenQuestion() {
  ui.deferred.clear();
  render();
}

async function dismissFailures() {
  const { files } = ui.sheet;
  await leaveSheet(() => command("dismiss_failures", { files }));
  focusSelectedTile();
}

async function retryFailures() {
  if (ui.leaving) return;
  const { files: shown } = ui.sheet;
  const retrying = ui.data.failures.filter((failure) => failure.retryable && shown.includes(failure.file));
  const names = unique(retrying.map((failure) => failure.slicer_name));
  const retried = await leaveSheet(() => command("retry_failures", { files: shown }));
  focusSelectedTile();
  if (retried && !failureShowing()) toast(`Trying ${files(retrying.length)} again in ${names.join(" and ")}`);
}

async function openElsewhere(slicerId) {
  if (ui.leaving) return;
  const { files: shown } = ui.sheet;
  const count = shown.length;
  const opened = await leaveSheet(() => command("open_failures_in", { files: shown, slicer: slicerId }));
  focusSelectedTile();
  if (opened && !failureShowing()) toast(`Opening ${files(count)} in ${slicerName(slicerId)}`);
}

// A modal keeps Tab inside the sheet and Escape closes it.
document.addEventListener("keydown", (event) => {
  const sheet = layer.querySelector(".sheet:not(.out)");
  if (!sheet || ui.leaving) return;
  if (event.key === "Escape") {
    event.preventDefault();
    dismissSheet();
  } else if (event.key === "Tab") {
    const focusable = [...sheet.querySelectorAll("button")];
    const index = focusable.indexOf(document.activeElement);
    const wrapsForward = !event.shiftKey && index === focusable.length - 1;
    const wrapsBack = event.shiftKey && index <= 0;
    if (index === -1 || wrapsForward || wrapsBack) {
      event.preventDefault();
      focusable[event.shiftKey ? focusable.length - 1 : 0]?.focus();
    }
  }
});

// New files can arrive while the window is open, and the window can sit behind
// others while the config changes, so refresh on the app's event and on focus.
listen("state-changed", refresh);
window.addEventListener("focus", refresh);
refresh();
