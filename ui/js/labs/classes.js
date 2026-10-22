import { store, subscribe } from "../store.js";
import { labOn } from "./util.js";

const CLASSES = {
  labs_hover: "labs-hover",
  labs_compact_sidebar: "labs-compact",
};

function apply() {
  for (const [key, name] of Object.entries(CLASSES)) {
    const on = key === "labs_compact_sidebar" ? store.settings[key] === true : labOn(key);
    document.documentElement.classList.toggle(name, on);
  }
}

subscribe("settings", apply);
