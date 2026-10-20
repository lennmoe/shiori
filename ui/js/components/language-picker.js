import { el } from "../dom.js";
import { t } from "../i18n.js";
import { store } from "../store.js";
import { call } from "../ui.js";

const flag = (code) => el("img", { class: "flag", src: `assets/flags/${code}.png`, alt: "" });

export function languagePicker(onPick) {
  const label = el("span");
  const button = el("button", { class: "btn secondary lang-button" }, label, " ▾");
  const list = el("div", { class: "menu lang-menu", role: "menu", hidden: true });
  const picker = el("div", { class: "lang-picker" }, button, list);

  button.onclick = () => (list.hidden = !list.hidden);
  document.addEventListener("click", (event) => {
    if (!picker.contains(event.target)) {
      list.hidden = true;
    }
  });

  call("languages").then((languages) => {
    const setting = store.settings.language || "auto";
    const choices = [
      { code: "auto", name: t("Same as Windows"), flag: "__" },
      ...languages,
    ];

    for (const choice of choices) {
      const item = el("button", { role: "menuitemradio" }, flag(choice.flag), choice.name);
      item.classList.toggle("on", choice.code === setting);
      item.onclick = () => {
        list.hidden = true;
        onPick(choice.code);
      };
      list.append(item);
      if (choice.code === setting) {
        label.replaceChildren(flag(choice.flag), choice.name);
      }
    }
  });

  return picker;
}
