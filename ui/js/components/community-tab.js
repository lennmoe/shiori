import { el, svg } from "../dom.js";
import { invoke } from "../tauri.js";
import { store } from "../store.js";
import { call, toast, toggle } from "../ui.js";
import { discordLogo } from "./discord-button.js";
import { COMMUNITY_URL } from "../links.js";
import { t } from "../i18n.js";

const EARTH =
  "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM4.2 9.3C5 7 6.6 5.2 8.7 4.3c.6.8.4 1.7-.4 2.3-.9.6-1.2 1.5-.6 2.4.5.8 1.6 1 2.3.4.9-.7 2.1-.4 2.5.6.5 1.1-.3 2.3-1.5 2.5-1.6.3-2.4 1.7-1.9 3.2.3.9 0 1.8-.8 2.3A8 8 0 0 1 4.2 9.3zm10.4 10.2c-.5-1-.3-2.1.6-2.8 1-.8 2.4-.7 3.3.2a8 8 0 0 1-3.9 2.6zm4.8-5.3c-1.1-.4-1.9-1.4-1.8-2.6.1-1.4-.9-2.4-2.2-2.6-1-.1-1.6-1-1.3-2 .2-.6.1-1.2-.3-1.6a8 8 0 0 1 5.6 8.8z";
const ICONS = {
  check: "M20 6 9 17l-5-5",
  cross: "M18 6 6 18M6 6l12 12",
  link: "M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7",
  open: "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
  feed: "M4 5h16M4 12h16M4 19h10",
};
const FRESH = 60000;

const SAVED_STATUS = "shiori.community.status";
const SAVED_STATS = "shiori.community.stats";

const remembered = (key) => {
  try {
    return JSON.parse(localStorage.getItem(key) || "null");
  } catch {
    return null;
  }
};
const remember = (key, value) => {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {}
};

let cached = remembered(SAVED_STATUS);
let checkedAt = 0;
let lastStats = remembered(SAVED_STATS);
let pending = null;
const loadedImages = new Set();

const open = (url) => call("open_url", { url });
const plain = (text) => document.createTextNode(text ?? "");

function icon(name, className = "cp-icon") {
  const box = svg("svg", { viewBox: "0 0 24 24", class: className, "aria-hidden": "true" });
  box.append(svg("path", name === "earth" ? { d: EARTH, "fill-rule": "evenodd" } : { d: ICONS[name] }));
  return box;
}

function handleOf(url) {
  const last = decodeURIComponent((url || "").split("/u/")[1] || "");
  return last ? `@${last.replace(/․/g, ".")}` : "";
}

async function libraryStats() {
  const games = await invoke("get_library").catch(() => []);
  const seconds = games.reduce((sum, game) => sum + (game.playtime || 0), 0);
  const read = games.filter((game) => (game.playtime || 0) >= 60 || game.status);
  lastStats = {
    hours: Math.round(seconds / 3600),
    vns: read.length,
    finished: games.filter((game) => game.status === "finished").length,
  };
  remember(SAVED_STATS, lastStats);
  return lastStats;
}

function imagesOf(status) {
  return [status?.profile?.avatar || status?.avatar_url, status?.profile?.banner].filter(Boolean);
}

function preload(status) {
  for (const url of imagesOf(status)) {
    if (!loadedImages.has(url)) {
      const probe = new Image();
      probe.onload = () => loadedImages.add(url);
      probe.src = url;
    }
  }
}

function fetchStatus() {
  pending ??= invoke("cloud_status")
    .then((status) => {
      cached = status;
      checkedAt = Date.now();
      remember(SAVED_STATUS, status);
      preload(status);
      return status;
    })
    .finally(() => (pending = null));
  return pending;
}

export function warmCommunity() {
  preload(cached);
  fetchStatus().catch(() => {});
  libraryStats().catch(() => {});
}

function statTile(label, value) {
  return el("div", { class: "cp-stat" }, el("span", {}, label), el("b", {}, value === null ? el("i", { class: "sk sk-num" }) : plain(value)));
}

function whenLoaded(url, show) {
  if (loadedImages.has(url)) {
    show(true);
    return;
  }
  const probe = new Image();
  probe.onload = () => {
    loadedImages.add(url);
    show(false);
  };
  probe.src = url;
}

function fadeIn(node, url, apply) {
  whenLoaded(url, (instant) => {
    apply();
    if (instant) {
      node.classList.add("ready", "instant");
    } else {
      requestAnimationFrame(() => node.classList.add("ready"));
    }
  });
}

function infoList(title, kind, lines) {
  return el(
    "div",
    { class: `cp-list ${kind}` },
    el("b", {}, title),
    lines.map((line) => el("div", { class: "cp-line" }, icon(kind === "shown" ? "check" : "cross", "cp-mark"), el("span", {}, line))),
  );
}

async function copy(url) {
  try {
    await navigator.clipboard.writeText(url);
    toast("Link copied");
  } catch {
    toast("Couldn't copy the link", { error: true });
  }
}

function linkVndb() {
  document.querySelector('[data-tab="library"]')?.click();
  requestAnimationFrame(() => document.querySelector('[data-section="vndb"]')?.scrollIntoView({ block: "start", behavior: "smooth" }));
}

function vndbNote() {
  return el(
    "div",
    { class: "cp-note cp-vndb" },
    el("span", {}, "Log in to VNDB so your ratings and reading statuses show up on your page."),
    el("button", { class: "btn small", onclick: linkVndb }, "Link VNDB"),
  );
}

export function communityTab() {
  const card = el("div", { class: "cp" });
  const tiles = (value) =>
    value
      ? [statTile(t("Time read"), `${value.hours}h`), statTile("VNs", String(value.vns)), statTile(t("Finished"), String(value.finished))]
      : [statTile(t("Time read"), null), statTile("VNs", null), statTile(t("Finished"), null)];
  const stats = el("div", { class: "cp-stats" }, ...tiles(lastStats));
  libraryStats().then((value) => stats.replaceChildren(...tiles(value)));

  const banner = (image = null) => {
    const layer = image ? el("div", { class: "cp-banner-img" }) : null;
    if (layer) {
      fadeIn(layer, image, () => (layer.style.backgroundImage = `url("${image.replace(/"/g, "%22")}")`));
    }
    return el(
      "div",
      { class: "cp-banner" },
      layer,
      el("span", { class: "cp-brand" }, icon("earth", "cp-earth"), "Shiori community"),
      el("button", { class: "btn secondary small cp-site", onclick: () => open(COMMUNITY_URL) }, icon("open"), "Open the website"),
    );
  };

  const place = (image, children) => {
    const appearing = card.classList.contains("loading");
    card.classList.remove("loading");
    card.replaceChildren(banner(image), ...children.filter(Boolean));
    if (appearing) {
      card.classList.remove("cp-in");
      void card.offsetWidth;
      card.classList.add("cp-in");
      setTimeout(() => card.classList.remove("cp-in"), 600);
    }
  };
  const shell = (...children) => place(null, children);
  const shellWith = (image, ...children) => place(image, children);

  const skeleton = () => {
    card.classList.add("loading");
    card.replaceChildren(
      banner(),
      el(
        "div",
        { class: "cp-head" },
        el("span", { class: "cp-avatar sk" }),
        el("div", { class: "cp-who" }, el("i", { class: "sk sk-line wide" }), el("i", { class: "sk sk-line" })),
      ),
      stats,
      el("div", { class: "cp-switch" }, el("i", { class: "sk sk-line long" })),
      el("div", { class: "cp-actions" }, el("i", { class: "sk sk-btn" }), el("i", { class: "sk sk-btn" }), el("i", { class: "sk sk-btn" })),
    );
  };

  const signedOut = () =>
    shell(
      el(
        "div",
        { class: "cp-head" },
        el("span", { class: "cp-avatar empty" }, discordLogo()),
        el(
          "div",
          { class: "cp-who" },
          el("b", {}, "Your reader page"),
          el("span", { class: "muted" }, "Follow other readers, see what they read and show your own library."),
        ),
      ),
      stats,
      el(
        "div",
        { class: "cp-actions" },
        el("button", { class: "btn discord-btn", onclick: signIn }, discordLogo(), "Sign in with Discord"),
        el("span", { class: "muted small-text" }, "You can browse the website without an account."),
      ),
    );

  const signedIn = (status) => {
    const shared = !!status.profile?.public;
    const url = status.profile?.url || "";
    const picture = status.profile?.avatar || status.avatar_url;
    const avatar = picture ? el("img", { class: "cp-avatar fade", alt: "" }) : el("span", { class: "cp-avatar empty" }, discordLogo());
    if (picture) {
      fadeIn(avatar, picture, () => (avatar.src = picture));
    }
    const pill = el(
      "span",
      { class: `cp-pill${shared ? " on" : ""}` },
      el("i", {}),
      shared ? "Public" : "Private",
    );
    const actions = shared && url
      ? el(
          "div",
          { class: "cp-actions" },
          el("button", { class: "btn", onclick: () => open(url) }, icon("open"), "My page"),
          el("button", { class: "btn secondary", onclick: () => copy(url) }, icon("link"), "Copy link"),
          el("button", { class: "btn secondary", onclick: () => open(COMMUNITY_URL) }, icon("feed"), "Open the feed"),
        )
      : el(
          "div",
          { class: "cp-note" },
          "Your page is hidden. Turn on Public profile to get it and show up in the feed.",
        );

    shellWith(
      status.profile?.banner || null,
      el(
        "div",
        { class: "cp-head" },
        avatar,
        el(
          "div",
          { class: "cp-who" },
          el("b", {}, plain(status.name || "Discord")),
          el("span", { class: "muted" }, plain(handleOf(url) || "Discord")),
        ),
        pill,
      ),
      stats,
      el(
        "div",
        { class: "cp-switch" },
        toggle(
          "Public profile",
          shared,
          async (on) => {
            try {
              await invoke("cloud_profile", { public: on });
              store.settings.profile_public = on;
              await fetchStatus();
              checkedAt = Date.now();
              signedIn(cached);
              toast(on ? "Your profile is public" : "Your profile is private");
            } catch (error) {
              toast(typeof error === "string" ? error : "Couldn't reach the cloud. Check your connection.", { error: true });
              signedIn(status);
            }
          },
          "Turn it off any time: your page and posts are hidden right away, and come back if you turn it on again.",
        ),
      ),
      actions,
      shared && !store.settings.vndb_token && vndbNote(),
    );
  };

  const draw = (status) => (status.signed_in ? signedIn(status) : signedOut());

  async function refresh() {
    try {
      const before = JSON.stringify(cached);
      const status = await fetchStatus();
      const changed = JSON.stringify(status) !== before;
      if (changed || card.classList.contains("loading") || card.querySelector(".hint-text")) {
        draw(status);
      }
    } catch {
      if (!cached) {
        shell(el("div", { class: "hint-text cp-wait" }, "Couldn't reach the cloud. Check your connection."));
      }
    }
  }

  async function signIn() {
    shell(el("div", { class: "hint-text cp-wait" }, "Finish signing in in your browser, then come back here."));
    try {
      cached = await invoke("cloud_login");
      remember(SAVED_STATUS, cached);
      checkedAt = Date.now();
      draw(cached);
      toast("Signed in");
    } catch (error) {
      toast(typeof error === "string" ? error : "Couldn't sign in", { error: true });
      cached = null;
      remember(SAVED_STATUS, null);
      refresh();
    }
  }

  if (cached) {
    draw(cached);
  } else {
    skeleton();
  }
  if (!cached || Date.now() - checkedAt > FRESH) {
    refresh();
  }

  const info = el(
    "div",
    { class: "cp-info" },
    infoList(t("What people see"), "shown", [
      t("The VN you're reading right now"),
      t("Your library with time read, statuses and ratings"),
      t("Posts, replies and likes you make on the website"),
    ]),
    infoList(t("Never shown"), "hidden", [
      t("VNs you hide on the website in their Privacy settings"),
      t("Covers marked as NSFW, unless the viewer chooses to see them"),
      t("Your screenshots, clips and settings"),
    ]),
  );

  return [card, info];
}
