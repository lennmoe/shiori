import { $, el, html } from "../dom.js";
import { fileUrl, pickFile } from "../tauri.js";
import { register } from "../router.js";
import { store, subscribe } from "../store.js";
import { call, confirmButton, toggle } from "../ui.js";

const TEMPLATE = `
  <div class="page-head">
    <h2>Mascots</h2>
    <div class="row">
      <span data-id="enabled"></span>
      <button class="btn" data-id="add">Add a mascot…</button>
    </div>
  </div>
  <p class="muted">
    Click a mascot to put it on your desktop, click it again to take it off. Pick several to have
    them all at once. On the desktop, drag them anywhere, click to chat, double-click to open the
    app, right-click for more. A PNG with a transparent background looks best.
  </p>
  <div class="mascot-grid" data-id="grid"></div>

  <div class="card mascot-options">
    <div class="card-title"><b>Options</b></div>
    <div data-id="options"></div>
  </div>
`;

const IMAGE_FILTERS = [{ name: "Images", extensions: ["png", "webp", "gif"] }];

let root;

const part = (id) => $(`[data-id="${id}"]`, root);

function build(container) {
  root = container;
  root.append(html(TEMPLATE));
  part("add").onclick = addMascot;
  subscribe("settings", () => root.classList.contains("active") && load());
}

const save = (changes) => call("update_settings", { changes });

function options() {
  const settings = store.settings;
  const backToCorner = el("button", { class: "btn secondary" }, "Put them back in the corner");
  backToCorner.onclick = () => save({ mascot_positions: {}, mascot_pos: null });

  return [
    toggle("Speech balloons", settings.mascot_talk, (on) => save({ mascot_talk: on })),
    toggle(
      "Keep them above other windows",
      settings.mascot_topmost,
      (on) => save({ mascot_topmost: on }),
      "Off: they stay on the desktop, behind your windows.",
    ),
    el("div", { class: "row" }, backToCorner),
  ];
}

async function load() {
  part("enabled").replaceChildren(
    toggle("Show on desktop", store.settings.mascot_enabled, (on) => save({ mascot_enabled: on })),
  );
  part("options").replaceChildren(...options());
  const items = await call("list_mascots");
  part("grid").replaceChildren(...items.map(card));
}

function card(item) {
  const picture = el("img", {
    src: item.built_in ? "assets/mascot.png" : fileUrl(item.file),
    alt: "",
    draggable: "false",
  });

  let remove = null;
  if (!item.built_in) {
    remove = el("button", { class: "btn danger small" });
    confirmButton(remove, "Remove", () => call("remove_mascot", { id: item.id }));
  }

  const flip = () => call("show_mascot", { id: item.id, shown: !item.shown });

  const sizeLabel = el("span", { class: "small-text" }, `Size: ${item.height}px`);
  const size = el("input", {
    type: "range",
    min: 120,
    max: 1200,
    step: 10,
    value: item.height,
    title: "Size",
  });
  size.onclick = (event) => event.stopPropagation();
  size.oninput = () => (sizeLabel.textContent = `Size: ${size.value}px`);
  size.onchange = () => call("set_mascot_size", { id: item.id, height: Number(size.value) });

  return el(
    "div",
    {
      class: `mascot-card ${item.shown ? "on" : ""}`,
      role: "button",
      tabindex: "0",
      onclick: flip,
      onkeydown: (event) => event.key === "Enter" && flip(),
    },
    el("div", { class: "mascot-pic" }, picture),
    el(
      "div",
      { class: "mascot-meta" },
      el("b", {}, item.name),
      el("span", { class: "small-text" }, item.shown ? "On your desktop" : "Click to show"),
      el("div", { class: "mascot-size" }, sizeLabel, size),
      remove,
    ),
  );
}

async function addMascot() {
  const path = await pickFile({ title: "Choose a mascot image", filters: IMAGE_FILTERS });
  if (path) {
    await call("add_mascot", { path });
  }
}

register({ id: "mascots", build, show: load });
