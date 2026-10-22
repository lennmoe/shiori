import { $, $$ } from "../dom.js";
import { labOn } from "./util.js";

document.addEventListener("keydown", (event) => {
  if (!labOn("labs_shortcuts") || $("dialog[open]") || $(".labs-search-overlay")) {
    return;
  }
  if (event.target.closest?.("input, textarea, select")) {
    return;
  }

  if (event.altKey && event.key === "ArrowLeft") {
    event.preventDefault();
    $(".page.active .back:not([hidden])")?.click();
    return;
  }

  if (!event.ctrlKey && !event.altKey && !event.metaKey && /^[1-9]$/.test(event.key)) {
    $$("nav button")[Number(event.key) - 1]?.click();
  }
});
