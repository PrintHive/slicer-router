const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

function show(id, visible) {
  $(id).hidden = !visible;
}

function render(state) {
  show("pending", state.pending.length > 0);
  $("pending-files").replaceChildren(
    ...state.pending.map((path) => Object.assign(document.createElement("li"), { textContent: path })),
  );

  show("notice", Boolean(state.notice));
  $("notice").textContent = state.notice ?? "";

  show("none", state.installed.length === 0);
  $("slicers").replaceChildren(
    ...state.installed.map((slicer) => {
      const label = document.createElement("label");
      label.className = "slicer";
      const radio = Object.assign(document.createElement("input"), {
        type: "radio",
        name: "default",
        value: slicer.id,
        checked: state.default_slicer === slicer.id,
      });
      radio.addEventListener("change", () => choose(slicer.id));
      const text = document.createElement("span");
      text.textContent = slicer.name;
      const path = document.createElement("small");
      path.textContent = slicer.path;
      text.append(path);
      label.append(radio, text);
      return label;
    }),
  );

  $("hint-mac").style.display = state.platform === "macos" ? "block" : "none";
  $("hint-windows").style.display = state.platform === "windows" ? "block" : "none";
}

async function choose(id) {
  try {
    await invoke("set_default", { slicer: id });
    $("status").textContent = "Saved.";
    render(await invoke("get_state"));
  } catch (error) {
    $("status").textContent = String(error);
  }
}

invoke("get_state").then(render, (error) => {
  $("status").textContent = String(error);
});
