const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

function show(id, visible) {
  $(id).hidden = !visible;
}

function element(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

// One banner per question. Files of a slicer that is not installed are asked
// about separately from files with no origin, because the answers are saved
// differently: an override for that slicer versus the default slicer.
function pendingBanner(group, installed) {
  const missing = group.detected_name;
  const heading = missing
    ? `${missing} isn't installed. Open these files with:`
    : "These files have no slicer metadata. Open them with:";
  const note = missing
    ? `Your choice is remembered for ${missing} files.`
    : "Your choice becomes the default slicer for files like these.";
  return element(
    "div",
    { className: "banner" },
    element("strong", { textContent: heading }),
    element("ul", {}, ...group.files.map((path) => element("li", { textContent: path }))),
    element(
      "div",
      { className: "choices" },
      ...installed.map((slicer) =>
        element("button", {
          textContent: slicer.name,
          onclick: () => choose(group.detected, slicer.id),
        }),
      ),
    ),
    element("p", { className: "muted note", textContent: note }),
  );
}

function render(state) {
  $("pending").replaceChildren(...state.pending.map((group) => pendingBanner(group, state.installed)));

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
      radio.addEventListener("change", () => choose(null, slicer.id));
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

async function choose(detected, id) {
  try {
    await invoke("choose", { detected, slicer: id });
    $("status").textContent = "Saved.";
    render(await invoke("get_state"));
  } catch (error) {
    $("status").textContent = String(error);
  }
}

invoke("get_state").then(render, (error) => {
  $("status").textContent = String(error);
});
