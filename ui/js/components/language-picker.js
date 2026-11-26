import { el } from "../dom.js";
import { t } from "../i18n.js";
import { store } from "../store.js";
import { call } from "../ui.js";

const flag = (code) => el("img", { class: "flag", src: `assets/flags/${code}.png`, alt: "" });

export function languagePicker(
  onPick,
  { current = store.settings.language || "auto", first = { code: "auto", name: "Same as Windows", flag: "__" }, right = false } = {},
) {
  const label = el("span");
  const button = el("button", { class: "btn secondary lang-button" }, label, " ▾");
  const list = el("div", { class: "menu lang-menu", role: "menu", popover: "manual" });
  const picker = el("div", { class: "lang-picker" }, button, list);
  const isOpen = () => list.matches(":popover-open");
  const close = () => isOpen() && list.hidePopover();
  const open = () => {
    list.showPopover();
    const box = button.getBoundingClientRect();
    const gap = 6;
    const below = window.innerHeight - box.bottom - gap - 8;
    const above = box.top - gap - 8;
    const wanted = list.scrollHeight;
    const up = wanted > below && above > below;
    list.style.maxHeight = `${Math.max(120, up ? above : below)}px`;
    const height = Math.min(wanted, parseFloat(list.style.maxHeight));
    list.style.top = `${up ? box.top - gap - height : box.bottom + gap}px`;
    const width = list.offsetWidth;
    const left = right ? box.right - width : box.left;
    list.style.left = `${Math.max(8, Math.min(left, window.innerWidth - width - 8))}px`;
  };

  button.onclick = () => (isOpen() ? close() : open());
  document.addEventListener("click", (event) => {
    if (!picker.contains(event.target)) {
      close();
    }
  });
  document.addEventListener("scroll", (event) => event.target !== list && close(), true);
  window.addEventListener("resize", close);

  call("languages").then((languages) => {
    const choices = [{ ...first, name: t(first.name) }, ...languages];
    const items = choices.map((choice) => {
      const item = el("button", { role: "menuitemradio" }, flag(choice.flag), choice.name);
      item.onclick = () => {
        close();
        select(choice.code);
        onPick(choice.code);
      };
      return item;
    });
    const select = (code) => {
      const index = Math.max(0, choices.findIndex((choice) => choice.code === code));
      items.forEach((item, i) => item.classList.toggle("on", i === index));
      label.replaceChildren(flag(choices[index].flag), choices[index].name);
    };
    list.append(...items);
    select(current);
  });

  return picker;
}
