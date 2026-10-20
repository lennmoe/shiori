import { $, el, html } from "../dom.js";
import { fileUrl, pickFile } from "../tauri.js";
import { register } from "../router.js";
import { store, subscribe } from "../store.js";
import { call, confirmButton, openDialog, toggle } from "../ui.js";
import { t } from "../i18n.js";

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

const CUSTOM = "custom";

const LINE_FIELDS = [
  ["start", "When you start a VN", "{name} becomes the VN's title."],
  ["stop", "When you stop reading"],
  ["idle", "When you go idle"],
  ["back", "When you come back"],
  ["section", "New chapter or route", "{section} becomes its name."],
  ["screenshot", "After a screenshot"],
];

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
  const [items, personalities] = await Promise.all([
    call("list_mascots"),
    call("mascot_personalities"),
  ]);
  part("grid").replaceChildren(...items.map((item) => card(item, personalities)));
}

function card(item, personalities) {
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

  const sizeText = (px) => t("Size: {px}px", { px });
  const sizeLabel = el("span", { class: "small-text" }, sizeText(item.height));
  const size = el("input", {
    type: "range",
    min: 120,
    max: 1200,
    step: 10,
    value: item.height,
    title: "Size",
  });
  size.onclick = (event) => event.stopPropagation();
  size.oninput = () => (sizeLabel.textContent = sizeText(size.value));
  size.onchange = () => call("set_mascot_size", { id: item.id, height: Number(size.value) });

  const personality = el(
    "select",
    { title: "Personality: changes what this mascot says" },
    ...[...personalities, { id: CUSTOM, name: t("Custom") }].map(({ id, name }) =>
      el("option", { value: id, selected: id === item.personality }, name),
    ),
  );
  const editLines = el("button", { class: "btn secondary small" }, "Write its lines…");
  editLines.hidden = item.personality !== CUSTOM;
  editLines.onclick = (event) => {
    event.stopPropagation();
    writeLines(item);
  };
  personality.onclick = (event) => event.stopPropagation();
  personality.onchange = () => {
    call("set_mascot_personality", { id: item.id, personality: personality.value });
    editLines.hidden = personality.value !== CUSTOM;
    if (personality.value === CUSTOM) {
      writeLines(item);
    }
  };

  return el(
    "div",
    {
      class: `mascot-card ${item.shown ? "on" : ""}`,
      role: "button",
      tabindex: "0",
      onclick: flip,
      onkeydown: (event) => {
        if (event.key === "Enter" && event.target === event.currentTarget) {
          flip();
        }
      },
    },
    el("div", { class: "mascot-pic" }, picture),
    el(
      "div",
      { class: "mascot-meta" },
      el("b", {}, item.name),
      el("span", { class: "small-text" }, item.shown ? "On your desktop" : "Click to show"),
      el("div", { class: "mascot-size" }, sizeLabel, size),
      el(
        "div",
        { class: "mascot-size" },
        el("span", { class: "small-text" }, "Personality"),
        personality,
        editLines,
      ),
      remove,
    ),
  );
}

async function writeLines(item) {
  const defaults = await call("personality_lines", { personality: "cheerful" });
  const saved = item.custom || {};

  const inputs = {};
  const fields = LINE_FIELDS.map(([key, label, hint]) => {
    inputs[key] = el("input", { type: "text", value: saved[key] || "", placeholder: defaults[key] });
    return el(
      "label",
      { class: "line-field" },
      el("span", {}, label),
      inputs[key],
      hint && el("span", { class: "small-text muted" }, hint),
    );
  });
  inputs.pokes = el("textarea", {
    rows: 4,
    placeholder: defaults.pokes.join("\n"),
  });
  inputs.pokes.value = (saved.pokes || []).join("\n");

  const answer = await openDialog({
    title: t("{name}'s lines", { name: item.name }),
    wide: true,
    body: el(
      "div",
      { class: "line-fields" },
      el(
        "p",
        { class: "muted small-text" },
        "Leave a line empty to keep the one shown in grey.",
      ),
      ...fields,
      el(
        "label",
        { class: "line-field" },
        el("span", {}, "When you click it"),
        inputs.pokes,
        el("span", { class: "small-text muted" }, "One per line, picked at random."),
      ),
    ),
    actions: [
      { label: "Cancel", kind: "secondary", value: null },
      { label: "Save", value: "save" },
    ],
  });
  if (answer !== "save") {
    return;
  }

  const lines = {};
  for (const [key] of LINE_FIELDS) {
    lines[key] = inputs[key].value.trim();
  }
  lines.pokes = inputs.pokes.value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  await call("set_mascot_custom_lines", { id: item.id, lines });
  item.custom = lines;
}

async function addMascot() {
  const path = await pickFile({ title: "Choose a mascot image", filters: IMAGE_FILTERS });
  if (path) {
    await call("add_mascot", { path });
  }
}

register({ id: "mascots", build, show: load });
