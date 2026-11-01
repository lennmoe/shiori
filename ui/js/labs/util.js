import { el } from "../dom.js";
import { call } from "../ui.js";

const VNDB = "https://vndb.org";

export const raw = (text) => document.createTextNode(text);

function link(target, text) {
  const url = target.startsWith("/") ? VNDB + target : target;
  const a = el("a", { href: "#" }, raw(text));
  a.onclick = (event) => {
    event.preventDefault();
    call("open_url", { url });
  };
  return a;
}

function spoiler(text) {
  const span = el("span", { class: "labs-spoiler", title: "Spoiler" }, raw(text));
  span.onclick = () => span.classList.add("shown");
  return span;
}

export function vndbText(source) {
  const out = [];
  const pattern = /\[url=([^\]]+)\]([\s\S]*?)\[\/url\]|\[spoiler\]([\s\S]*?)(?:\[\/spoiler\]|$)|\[\/?(?:b|i|u|s|raw|quote|code)\]/gi;
  let last = 0;
  for (const match of source.matchAll(pattern)) {
    out.push(raw(source.slice(last, match.index)));
    if (match[1]) {
      out.push(link(match[1], match[2]));
    } else if (match[3] !== undefined) {
      out.push(spoiler(match[3].trim()));
    }
    last = match.index + match[0].length;
  }
  out.push(raw(source.slice(last)));
  return el("div", { class: "labs-description" }, ...out);
}
