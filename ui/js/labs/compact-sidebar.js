import { $, $$, el } from "../dom.js";
import { store } from "../store.js";
import { call } from "../ui.js";

const ICONS = {
  home: "M4 5h7a2 2 0 0 1 2 2v12a2 2 0 0 0-2-2H4zM20 5h-7a2 2 0 0 0-2 2v12a2 2 0 0 1 2-2h7z",
  library: "M4 4h4v16H4zM10 4h4v16h-4zM16 5l4 1-3 14-4-1z",
  screenshots: "M4 6h16v12H4zM8 14h8M8 10h3M17 3l.8 1.7 1.7.8-1.7.8L17 8l-.8-1.7-1.7-.8 1.7-.8z",
  share: "M18 8a3 3 0 1 0-3-3M6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.6 13.5l6.8 4M15.4 6.5l-6.8 4",
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
