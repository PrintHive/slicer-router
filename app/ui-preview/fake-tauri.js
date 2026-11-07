// Stands in for the Tauri runtime so ../ui runs in a browser, on fixture states.
// Dev only: this folder is outside frontendDist and never ships in the app.
//
// frame.html?state=<scenario>&icons=none&launch=fail&makeDefault=fail
(() => {
  const params = new URLSearchParams(location.search);
  const SLICERS = {
    "bambu-studio": { name: "Bambu Studio", path: "/Applications/BambuStudio.app", color: "#1fa84f" },
    "orca-slicer": { name: "OrcaSlicer", path: "/Applications/OrcaSlicer.app", color: "#3b82f6" },
    "snapmaker-orca": { name: "Snapmaker Orca", path: "/Applications/Snapmaker Orca.app", color: "#c084fc" },
    "prusa-slicer": { name: "PrusaSlicer", path: "/Applications/Original Prusa Drivers/PrusaSlicer.app", color: "#f97316" },
  };
  const ALL = Object.keys(SLICERS);
  const installedOf = (ids) => ids.map((id) => ({ id, name: SLICERS[id].name, path: SLICERS[id].path }));
  const downloads = (names) => names.map((name) => `/Users/you/Downloads/${name}`);

  const NO_INFO = downloads(["Ultimate_Ironing_Test_v3_final_FINAL (2).3mf", "plain-cube-no-metadata.3mf"]);
  const PRUSA = downloads([
    "Zou_Ball_Python_Snake_Straight_0.4mm_PLA_MK4S_2h13m.3mf",
    "benchy_MK4S.3mf",
    "xl-toolchanger-calibration.3mf",
  ]);
  const MANY = Array.from(
    { length: 14 },
    (_, i) => `/Users/you/Downloads/printables-batch/part_${String(i + 1).padStart(2, "0")}_${["bracket", "hinge", "knob", "clip", "spacer", "lid", "foot"][i % 7]}.3mf`,
  );

  const failure = (file, id, reason, retryable = true) => ({
    file,
    slicer: id,
    slicer_name: id ? SLICERS[id].name : null,
    reason,
    retryable,
  });
  const didNotStart = (id) => `${SLICERS[id].name} didn't start (exit status 1)`;

  const base = (changes = {}) => ({
    platform: "macos",
    installed: installedOf(ALL),
    default_slicer: "bambu-studio",
    overrides: {},
    default_app: { is_default: true, current_name: "Slicer Router" },
    pending: [],
    failures: [],
    config_path: "/Users/you/Library/Application Support/Slicer Router/config.json",
    ...changes,
  });
  const notDefault = { is_default: false, current_name: "Bambu Studio" };

  const SCENARIOS = {
    default: () => base(),
    overrides: () => base({ overrides: { "bambu-studio": "orca-slicer", "prusa-slicer": "snapmaker-orca" } }),
    "mac-not-default": () => base({ default_app: notDefault }),
    "windows-not-default": () => base({ platform: "windows", default_app: notDefault }),
    "status-unknown": () => base({ default_app: null }),
    "files-waiting": () =>
      base({ default_slicer: null, pending: [{ detected: null, detected_name: null, files: NO_INFO }] }),
    "missing-slicer": () =>
      base({
        installed: installedOf(ALL.filter((id) => id !== "prusa-slicer")),
        pending: [{ detected: "prusa-slicer", detected_name: "PrusaSlicer", files: PRUSA }],
      }),
    "many-files": () =>
      base({ default_slicer: null, pending: [{ detected: null, detected_name: null, files: MANY }] }),
    "two-questions": () =>
      base({
        default_slicer: null,
        installed: installedOf(ALL.filter((id) => id !== "prusa-slicer")),
        pending: [
          { detected: null, detected_name: null, files: NO_INFO },
          { detected: "prusa-slicer", detected_name: "PrusaSlicer", files: PRUSA },
        ],
      }),
    failure: () => base({ failures: [failure(PRUSA[0], "prusa-slicer", didNotStart("prusa-slicer"))] }),
    failures: () =>
      base({
        failures: [
          failure(PRUSA[0], "prusa-slicer", didNotStart("prusa-slicer")),
          failure(PRUSA[1], "prusa-slicer", didNotStart("prusa-slicer")),
          failure("/Users/you/Documents/Prints/H2D_dual_nozzle_organizer_tray_v7.3mf", "bambu-studio", "Bambu Studio was moved or deleted", false),
        ],
      }),
    "failure-and-waiting": () =>
      base({
        default_slicer: null,
        pending: [{ detected: null, detected_name: null, files: NO_INFO }],
        failures: [failure(PRUSA[0], "prusa-slicer", didNotStart("prusa-slicer"))],
      }),
    "missing-default": () =>
      base({ installed: installedOf(ALL.filter((id) => id !== "prusa-slicer")) }),
    "missing-ask": () =>
      base({ installed: installedOf(ALL.filter((id) => id !== "prusa-slicer")), default_slicer: null }),
    "single-slicer": () => base({ installed: installedOf(["orca-slicer"]), default_slicer: null }),
    empty: () => base({ installed: [], default_slicer: null }),
    "no-slicer": () =>
      base({
        installed: [],
        default_slicer: null,
        failures: [failure(PRUSA[1], null, "No supported slicer is installed", false)],
      }),
  };

  const scenario = params.get("state") in SCENARIOS ? params.get("state") : "default";
  const model = SCENARIOS[scenario]();
  const listeners = [];
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const emit = () => setTimeout(() => listeners.forEach((listener) => listener({ payload: null })), 0);
  const launchFails = params.get("launch") === "fail";

  const launchIn = (files, id) =>
    launchFails ? files.map((file) => failure(file, id, didNotStart(id))) : [];

  const handlers = {
    get_state: () => structuredClone(model),
    slicer_icons: () =>
      Object.fromEntries(
        model.installed.map(({ id }) => [id, params.get("icons") === "none" ? null : placeholderIcon(id)]),
      ),
    async choose({ detected, slicer }) {
      await wait(400);
      const released = model.pending.filter((group) => group.detected === detected);
      model.pending = model.pending.filter((group) => group.detected !== detected);
      if (detected === null) model.default_slicer = slicer;
      else model.overrides[detected] = slicer;
      model.failures.push(...released.flatMap((group) => launchIn(group.files, slicer)));
      emit();
    },
    set_override({ detected, slicer }) {
      if (slicer === null || slicer === detected) delete model.overrides[detected];
      else model.overrides[detected] = slicer;
    },
    make_default_app() {
      if (params.get("makeDefault") === "fail") {
        throw "macOS does not know Slicer Router yet. Copy it to Applications, open it from there, and try again.";
      }
      model.default_app = { is_default: true, current_name: "Slicer Router" };
    },
    reveal_example_3mf() {},
    async retry_failures({ files }) {
      await wait(400);
      model.failures = model.failures.filter((f) => !(files.includes(f.file) && f.retryable));
      emit();
    },
    async open_failures_in({ files, slicer }) {
      await wait(400);
      const opened = model.failures.filter((f) => files.includes(f.file));
      model.failures = model.failures.filter((f) => !files.includes(f.file));
      model.failures.push(...opened.flatMap((f) => launchIn([f.file], slicer)));
      emit();
    },
    dismiss_failures({ files }) {
      model.failures = model.failures.filter((f) => !files.includes(f.file));
    },
  };

  async function invoke(name, args = {}) {
    await wait(40);
    const handler = handlers[name];
    if (!handler) throw `fake-tauri: no handler for ${name}`;
    return handler(args);
  }

  // Not the slicers' artwork: a lettered square in a made-up colour.
  function placeholderIcon(id) {
    const { name, color } = SLICERS[id];
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><rect width="128" height="128" rx="28" fill="${color}"/><text x="64" y="88" font-family="sans-serif" font-size="72" font-weight="700" text-anchor="middle" fill="#fff">${name[0]}</text></svg>`;
    return `data:image/svg+xml;base64,${btoa(svg)}`;
  }

  window.__TAURI__ = {
    core: { invoke },
    event: {
      listen: async (_name, callback) => {
        listeners.push(callback);
        return () => {};
      },
    },
  };
  window.__PREVIEW__ = { scenarios: Object.keys(SCENARIOS) };
})();
