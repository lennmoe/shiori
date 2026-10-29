import { el } from "../dom.js";
import { show } from "../router.js";
import { store, subscribe } from "../store.js";
import { call, openDialog, setCover, toast } from "../ui.js";
import { t } from "../i18n.js";

const asked = new Set();
let open = false;

function row(label, ...value) {
  return el("div", { class: "nv-row" }, el("span", { class: "nv-label" }, label), el("div", { class: "nv-value" }, ...value));
}

function processBlock(snap) {
  return el(
    "div",
    { class: "nv-block" },
    el("div", { class: "nv-title" }, "What was detected"),
    row("Program", el("code", {}, snap.exe || "—")),
    snap.engine_name && row("Engine", snap.engine_name),
    snap.raw_title && row("Window", el("span", { class: "nv-window" }, snap.raw_title)),
  );
}

function vnBlock(snap) {
  const vn = snap.vn;
  const cover = el("div", { class: "cover nv-cover" });
  const cv = snap.cover || {};
  setCover(cover, { path: cv.local_path, nsfw: cv.nsfw, label: vn?.title || snap.game_name });

  const info = vn
    ? el(
        "div",
        { class: "nv-info" },
        el("b", { class: "nv-name" }, vn.title),
        vn.alt_title && vn.alt_title !== vn.title && el("span", { class: "muted" }, vn.alt_title),
        el("span", { class: "muted small-text" }, [vn.year?.slice(0, 4), vn.id].filter(Boolean).join("  ·  ")),
        el(
          "button",
          { class: "link nv-link", onclick: () => call("open_url", { url: `https://vndb.org/${vn.id}` }) },
          "See it on VNDB",
        ),
      )
    : el(
        "div",
        { class: "nv-info" },
        el("b", { class: "nv-name" }, snap.game_name || snap.raw_title),
        el("span", { class: "muted" }, "Not found on VNDB."),
        el("span", { class: "muted small-text" }, "Pick it yourself to get its name, cover and length."),
      );

  return el(
    "div",
    { class: "nv-block" },
    el("div", { class: "nv-title" }, vn ? "Matched on VNDB" : "VNDB"),
    el("div", { class: "nv-vn" }, cover, info),
  );
}

async function ask(snap) {
  open = true;
  asked.add(snap.key);
  const answer = await openDialog({
    title: "New VN detected",
    wide: true,
    body: el(
      "div",
      { class: "nv" },
      el("p", { class: "muted" }, "Check that it's the right game: this name and cover are what your friends see on Discord. Your answer is remembered for this game."),
      el("div", { class: "nv-grid" }, processBlock(snap), vnBlock(snap)),
    ),
    actions: [
      !snap.emulator && { label: "Not a VN", kind: "danger", value: "not-vn" },
      { label: "Later", kind: "secondary", value: null },
      { label: snap.vn ? "Pick another VN…" : "Pick it on VNDB…", kind: snap.vn ? "secondary" : "", value: "pick" },
      snap.vn && { label: "Yes, that's it", value: "yes" },
    ].filter(Boolean),
  });
  open = false;

  if (answer === "yes") {
    await call("confirm_match", { key: snap.key });
    toast(t("Saved: {name}", { name: snap.vn.title }));
  } else if (answer === "pick") {
    await call("match_checked", { key: snap.key });
    show("cover", { key: snap.key });
  } else if (answer === "not-vn") {
    await call("match_checked", { key: snap.key });
    await call("not_a_vn", { exe: snap.exe });
    toast(t("{exe} won't be detected again", { exe: snap.exe }));
  }
}

function check(snap) {
  if (snap?.detected && snap.needs_check && !open && !asked.has(snap.key)) {
    ask(snap);
  }
}

export function watchNewVns() {
  subscribe("snapshot", check);
  check(store.snapshot);
}
