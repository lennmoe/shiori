import { el } from "../dom.js";
import { invoke } from "../tauri.js";
import { store } from "../store.js";
import { openDialog, toast } from "../ui.js";
import { t } from "../i18n.js";

export async function offerUpdate({ quiet = true } = {}) {
  let release;
  try {
    release = await invoke("check_update");
  } catch {
    if (!quiet) {
      toast("Couldn't reach GitHub to check for updates (are you offline?).", { error: true });
    }
    return "failed";
  }

  if (!release) {
    if (!quiet) {
      toast(t("You're up to date (version {version}).", { version: store.version }));
    }
    return "up-to-date";
  }

  const answer = await openDialog({
    title: "Update available",
    body: el(
      "p",
      { class: "muted" },
      t("Visual Novel RPC {latest} is out (you have {version}). Download it and restart now?", {
        latest: release.version,
        version: store.version,
      }),
    ),
    actions: [
      { label: "Later", kind: "secondary", value: false },
      { label: "Update", value: true },
    ],
  });
  if (answer !== true) {
    return "update";
  }

  toast(t("Downloading version {version}…", { version: release.version }));
  try {
    await invoke("install_update", { release });
  } catch (error) {
    toast(t("Update failed: {error}", { error }), { error: true });
  }
  return "update";
}
