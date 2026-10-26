import { invoke, listen } from "../tauri.js";
import { followTheme } from "./theme.js";

const balloon = document.getElementById("balloon");
let saying = 0;

async function say(text) {
  const mine = ++saying;
  balloon.classList.remove("shown");
  balloon.textContent = text;
  if (!text) {
    return;
  }
  await document.fonts.ready;

  const style = getComputedStyle(balloon);
  const marginX = parseFloat(style.marginLeft) + parseFloat(style.marginRight);
  const marginY = parseFloat(style.marginTop) + parseFloat(style.marginBottom);
  const scale = window.devicePixelRatio || 1;
  const width = Math.ceil((balloon.offsetWidth + marginX + 4) * scale);
  const height = Math.ceil((balloon.offsetHeight + marginY + 4) * scale);

  const side = await invoke("place_balloon", { width, height });
  if (mine !== saying) {
    return;
  }
  document.body.classList.toggle("right", side === "right");
  requestAnimationFrame(() => balloon.classList.add("shown"));
}

function hush() {
  saying += 1;
  balloon.classList.remove("shown");
}

balloon.addEventListener("click", () => invoke("hide_balloon"));

async function start() {
  await followTheme();
  await listen("say", (event) => say(event.payload));
  await listen("hush", hush);
  say(await invoke("balloon_text"));
}

start();
