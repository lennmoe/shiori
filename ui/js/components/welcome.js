import { el } from "../dom.js";
import { store, subscribe } from "../store.js";
import { call, openDialog, toggle } from "../ui.js";
import { languagePicker } from "./language-picker.js";
import { t } from "../i18n.js";

function step(number, title, ...content) {
  return el(
    "div",
    { class: "welcome-step" },
    el("span", { class: "welcome-number" }, String(number)),
    el("div", { class: "welcome-text" }, el("b", {}, title), ...content),
  );
}

function discordLine() {
  const line = el("span", { class: "muted" });
  const render = () => {
    const ok = !!store.status.discord_ok;
    line.className = ok ? "ok-text" : "muted";
    line.textContent = ok
      ? t("Connected to Discord ✓")
      : t("Open the Discord app (not the website). It connects by itself.");
  };
  render();
  subscribe("status", () => line.isConnected && render());
  return line;
}

export async function welcome() {
  const picker = languagePicker(async (language) => {
    await call("update_settings", { changes: { language } });
    location.reload();
  });
  const autostart = toggle("Launch when Windows starts", store.autostart, async (on) => {
    await call("set_autostart", { enabled: on });
    store.autostart = on;
  });

  await openDialog({
    title: "Welcome to Shiori RPC",
    body: el(
      "div",
      { class: "welcome" },
      el(
        "p",
        { class: "muted" },
        "It shows the visual novel you're reading on your Discord profile and keeps track of your time read.",
      ),
      step(1, "Language", picker),
      step(2, "Discord", discordLine()),
      step(
        3,
        "Start a VN",
        el(
          "span",
          { class: "muted" },
          "Most games are found by themselves. The first time, you're asked to check it's the right VN.",
        ),
      ),
      autostart,
    ),
    actions: [{ label: "Let's go", value: true }],
  });
  await call("update_settings", { changes: { welcome_done: true } });
}
