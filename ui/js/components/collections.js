import { el } from "../dom.js";
import { store } from "../store.js";
import { pickFolder } from "../tauri.js";
import { call, openDialog } from "../ui.js";
import { t } from "../i18n.js";

export const collectionList = () =>
  (Array.isArray(store.settings.collections) ? store.settings.collections : []).filter((c) => c?.id);

export const collectionById = (id) => collectionList().find((c) => c.id === id);

export async function askName(title, value = "") {
  const input = el("input", { type: "text", maxlength: 40, placeholder: "Name", class: "coll-name" });
  input.value = value;
  const name = await openDialog({
    title,
    body: input,
    actions: [
      { label: "Cancel", kind: "secondary", value: null },
      { label: "Save", value: "save" },
    ],
    onOpen: (finish) => {
      input.focus();
      input.select();
      input.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && input.value.trim()) {
          finish("save");
        }
      });
    },
  });
  return name === "save" ? input.value.trim() : null;
}

export async function newCollection() {
  const name = await askName(t("New collection"));
  return name ? call("save_collection", { name }) : null;
}

export async function chooseFolder(title, current) {
  return pickFolder({ title: t(title), defaultPath: current || undefined });
}

export async function pickGames(collection, games) {
  const chosen = new Set(games.filter((g) => g.collections?.includes(collection.id)).map((g) => g.key));
  const filter = el("input", { type: "search", placeholder: "Filter…" });
  const list = el("div", { class: "coll-pick" });
  const count = el("span", { class: "muted small-text" });
  const sorted = [...games].sort((a, b) => a.name.localeCompare(b.name));

  const draw = () => {
    const text = filter.value.trim().toLowerCase();
    count.textContent = t("{n} selected", { n: chosen.size });
    list.replaceChildren(
      ...sorted
        .filter((game) => !text || game.name.toLowerCase().includes(text))
        .map((game) => {
          const box = el("input", { type: "checkbox", checked: chosen.has(game.key) });
          box.onchange = () => {
            if (box.checked) chosen.add(game.key);
            else chosen.delete(game.key);
            count.textContent = t("{n} selected", { n: chosen.size });
          };
          return el("label", { class: "coll-pick-row" }, box, el("span", {}, game.name));
        }),
    );
  };
  filter.oninput = draw;
  draw();

  const answer = await openDialog({
    title: t("Games in {name}", { name: collection.name }),
    body: el("div", { class: "coll-pick-body" }, el("div", { class: "row" }, filter, count), list),
    width: 520,
    actions: [
      { label: "Cancel", kind: "secondary", value: null },
      { label: "Save", value: "save" },
    ],
    onOpen: () => filter.focus(),
  });
  if (answer !== "save") {
    return false;
  }

  await call("set_collection_games", { id: collection.id, keys: [...chosen] });
  return true;
}
