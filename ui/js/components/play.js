import { invoke, pickFile } from "../tauri.js";
import { ask, call, toast } from "../ui.js";
import { t } from "../i18n.js";

export const LAUNCHERS = {
  "": { label: "Normal" },
  le: { label: "Locale Emulator", exe: "LEProc.exe" },
  ntlea: { label: "NTLEA", exe: "ntleas.exe" },
};

export async function locateGame(key) {
  const path = await pickFile({
    title: "Locate the game's executable",
    filters: [{ name: "Programs", extensions: ["exe"] }],
  });
  if (!path) {
    return false;
  }
  await call("set_game_path", { key, path });
  return true;
}

export async function locateTool(launcher) {
  const tool = LAUNCHERS[launcher];
  const path = await pickFile({
    title: `Locate ${tool.exe} (${tool.label})`,
    filters: [{ name: tool.exe, extensions: ["exe"] }],
  });
  if (!path) {
    return false;
  }
  await call("set_tool_path", { launcher, path });
  return true;
}

export async function playGame(key) {
  try {
    await invoke("play_game", { key });
    toast("Starting…");
  } catch (error) {
    await launchFailed(key, error);
  }
}

async function launchFailed(key, error) {
  if (error?.kind === "GameMissing") {
    const locate = await ask(
      "Play",
      "This game's executable can't be found anymore (it may have moved or been uninstalled). Locate it?",
      { yes: "Locate…" },
    );
    if (locate && (await locateGame(key))) {
      await playGame(key);
    }
    return;
  }

  if (error?.kind === "ToolMissing") {
    const tool = LAUNCHERS[error.detail];
    const message = t("{tool} ({exe}) wasn't found. Locate it?", { tool: tool.label, exe: tool.exe });
    const locate = await ask("Play", message, {
      yes: "Locate…",
    });
    if (locate && (await locateTool(error.detail))) {
      await playGame(key);
    }
    return;
  }

  toast(t("Couldn't launch it: {error}", { error: error?.detail || error }), { error: true });
}
