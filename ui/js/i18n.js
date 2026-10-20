const ATTRIBUTES = ["title", "placeholder", "aria-label"];

let dictionary = null;
export let language = "en";

export async function loadLanguage(code) {
  language = "en";
  dictionary = null;
  if (code && code !== "en") {
    try {
      const response = await fetch(`locales/${code}.json`);
      dictionary = (await response.json()).ui || null;
      language = code;
    } catch {
      dictionary = null;
    }
  }
  document.documentElement.lang = language;
}

const squash = (text) => text.replace(/\s+/g, " ").trim();

export function t(text, vars) {
  let out = text;
  if (dictionary && typeof text === "string") {
    out = dictionary[text] ?? dictionary[squash(text)] ?? text;
  }
  if (vars) {
    out = out.replace(/\{(\w+)\}/g, (match, name) => (name in vars ? vars[name] : match));
  }
  return out;
}

function translateText(value) {
  const key = squash(value);
  const hit = key && dictionary[key];
  if (!hit) {
    return value;
  }
  const lead = value.match(/^\s*/)[0];
  const tail = value.match(/\s*$/)[0];
  return lead + hit + tail;
}

export function translateTree(root) {
  if (!dictionary) {
    return root;
  }
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    node.nodeValue = translateText(node.nodeValue);
  }
  for (const node of root.querySelectorAll("[title], [placeholder], [aria-label]")) {
    for (const name of ATTRIBUTES) {
      if (node.hasAttribute(name)) {
        node.setAttribute(name, translateText(node.getAttribute(name)));
      }
    }
  }
  return root;
}

export function plural(count, word) {
  const form = count === 1 ? word : `${word}s`;
  return `${count} ${t(form)}`;
}
