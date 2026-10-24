import { $, $$, el } from "../dom.js";
import { store } from "../store.js";
import { call } from "../ui.js";

const ICONS = {
  home: "M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1z",
  library: "M3 20h18M5 20V8h3v12M10 20V5h3v15M15 20l1-11 3 .4-1 10.6",
  screenshots: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM9 11.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 16l-5-5-9 9",
  share: "M4 20V10M10 20V4M16 20v-7M20 20H3",
  mascots: "M12 14a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM8.3 6.6 7.5 2.5l3 2.2M15.7 6.6l.8-4.1-3 2.2M10.5 9.5h.01M13.5 9.5h.01M5 21a7 7 0 0 1 14 0",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19 12a7 7 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6A7 7 0 0 0 5 12a7 7 0 0 0 .1 1.2l-2 1.6 2 3.4 2.4-1a7 7 0 0 0 2 1.2L10 21h4l.5-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6A7 7 0 0 0 19 12z",
};

function icon(path) {
  const box = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  box.setAttribute("viewBox", "0 0 24 24");
  box.setAttribute("class", "labs-nav-icon");
  const line = document.createElementNS("http://www.w3.org/2000/svg", "path");
  line.setAttribute("d", path);
  box.append(line);
  return box;
}

for (const button of $$("nav button")) {
  const label = el("span", { class: "labs-nav-text" });
  label.append(...button.childNodes);
  button.title = label.textContent.trim();
  button.append(icon(ICONS[button.dataset.page] || ICONS.home), label);
}

const toggle = el(
  "button",
  { class: "btn secondary labs-compact-toggle", title: "Compact sidebar" },
  icon("M4 4h16v16H4zM9 4v16"),
);
const showArrow = () =>
  toggle.classList.toggle("on", document.documentElement.classList.contains("labs-compact"));
toggle.onclick = async () => {
  const compact = !document.documentElement.classList.contains("labs-compact");
  document.documentElement.classList.toggle("labs-compact", compact);
  showArrow();
  store.settings.labs_compact_sidebar = compact;
  await call("update_settings", { changes: { labs_compact_sidebar: compact } });
};
$(".side-bottom")?.append(toggle);
new MutationObserver(showArrow).observe(document.documentElement, { attributeFilter: ["class"] });
showArrow();
