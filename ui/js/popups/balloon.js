import { invoke, listen } from "../tauri.js";
import { followTheme } from "./theme.js";

const balloon = document.getElementById("balloon");

function say(text) {
  balloon.textContent = text;

  balloon.style.animation = "none";
  void balloon.offsetWidth;
  balloon.style.animation = "";

  retries = 0;
  requestAnimationFrame(place);
}

let retries = 0;

function overflowing() {
  const box = balloon.getBoundingClientRect();
  return box.right + 2 > window.innerWidth || box.bottom + 2 > window.innerHeight;
}

window.addEventListener("resize", () => {
  if (overflowing() && retries < 3) {
    retries += 1;
    place();
  }
});

async function place() {
  await document.fonts.ready;

  const style = getComputedStyle(balloon);
  const marginX = parseFloat(style.marginLeft) + parseFloat(style.marginRight);
  const marginY = parseFloat(style.marginTop) + parseFloat(style.marginBottom);

  const scale = window.devicePixelRatio || 1;
  const width = Math.ceil((balloon.offsetWidth + marginX + 4) * scale);
  const height = Math.ceil((balloon.offsetHeight + marginY + 4) * scale);
  invoke("place_balloon", { width, height });
}

balloon.addEventListener("click", () => invoke("hide_balloon"));

async function start() {
  await followTheme();
  await listen("say", (event) => say(event.payload));
  say(await invoke("balloon_text"));
}

start();
