import { el } from "../dom.js";
import { call, openDialog, setCover } from "../ui.js";
import { plural, t } from "../i18n.js";
import { shortDuration } from "../format.js";

const STATUS = { playing: "Reading", finished: "Finished", stalled: "Stalled", dropped: "Dropped", "": "No status" };

const FILTERS = [
  ["changed", "Different"],
  ["cloud", "Only in the cloud"],
  ["here", "Only on this PC"],
  ["all", "All"],
];

const time = (seconds) => (seconds ? shortDuration(seconds) : "0m");

function tile(label, cloud, here, format = String) {
  const same = cloud === here;
  return el(
    "div",
    { class: "cprev-tile" },
    el("span", { class: "cprev-tile-label" }, t(label)),
    el("b", { class: "cprev-tile-value" }, format(cloud)),
    el("span", { class: `cprev-tile-now${same ? "" : " diff"}` }, t("Now: {value}", { value: format(here) })),
  );
}

function side(info) {
  if (!info) {
    return "—";
  }
  const status = info.status ? t(STATUS[info.status] ?? info.status) : "";
  return [time(info.seconds), status].filter(Boolean).join(" · ");
}

const BADGES = {
  cloud: ["New", "add"],
  here: ["Removed", "remove"],
  changed: ["Changed", "change"],
};

function row(item) {
  const cover = el("div", { class: "cover cprev-cover" });
  setCover(cover, { path: item.cover || "", nsfw: item.nsfw, label: item.name });
  const badge = BADGES[item.state];
  const detail =
    item.state === "changed"
      ? el("span", { class: "cprev-detail" }, el("s", {}, side(item.here)), " → ", side(item.cloud))
      : el("span", { class: "cprev-detail" }, side(item.cloud ?? item.here));
  return el(
    "div",
    { class: `cprev-row ${item.state}`, "data-state": item.state },
    cover,
    el("div", { class: "cprev-main" }, el("b", { class: "cprev-name" }, item.name), detail),
    badge ? el("span", { class: `cprev-badge ${badge[1]}` }, t(badge[0])) : null,
  );
}

function render(preview) {
  const items = [...preview.items].sort(
    (a, b) => (b.cloud?.seconds ?? b.here?.seconds ?? 0) - (a.cloud?.seconds ?? a.here?.seconds ?? 0),
  );
  const count = (state) => (state === "all" ? items.length : items.filter((item) => item.state === state).length);
  const list = el("div", { class: "cprev-list" });
  const empty = el("div", { class: "muted cprev-empty" }, t("Nothing here."));

  const pick = (value) => {
    for (const chip of chips) chip.classList.toggle("on", chip.dataset.value === value);
    const shown = items.filter((item) => value === "all" || item.state === value);
    list.replaceChildren(...(shown.length ? shown.map(row) : [empty]));
  };
  const chips = FILTERS.filter(([value]) => value === "all" || count(value)).map(([value, label]) => {
    const chip = el(
      "button",
      { class: "pull-chip", type: "button", "data-value": value },
      t(label),
      el("span", { class: "cprev-count" }, String(count(value))),
    );
    chip.onclick = () => pick(value);
    return chip;
  });
  pick(chips[0].dataset.value);

  const removed = count("here");
  return el(
    "div",
    { class: "cprev" },
    el(
      "div",
      { class: "cprev-tiles" },
      tile("VNs", preview.cloud.vns, preview.here.vns),
      tile("Time read", preview.cloud.seconds, preview.here.seconds, time),
      tile("Finished", preview.cloud.finished, preview.here.finished),
    ),
    removed
      ? el(
          "div",
          { class: "cprev-warn" },
          t("{vns} on this PC aren't in the cloud save and will be removed.", { vns: plural(removed, "VN") }),
        )
      : null,
    preview.settings_changed
      ? el(
          "div",
          { class: "muted cprev-note" },
          t("Your settings change too ({settings}).", { settings: plural(preview.settings_changed, "setting") }),
        )
      : null,
    el("div", { class: "pull-chips" }, chips),
    list,
  );
}

export async function previewCloud(button, { date, folder }) {
  const label = button.textContent;
  button.disabled = true;
  button.textContent = t("Reading your cloud save…");
  let preview;
  try {
    preview = await call("cloud_preview");
  } catch {
    return false;
  } finally {
    button.disabled = false;
    button.textContent = label;
  }
  if (!preview) {
    return false;
  }
  const answer = await openDialog({
    title: t("Cloud save from {date}", { date }),
    body: el(
      "div",
      {},
      render(preview),
      el("p", { class: "muted cprev-foot" }, t("What you have now is saved first in {folder}.", { folder })),
    ),
    wide: true,
    actions: [
      { label: "Cancel", kind: "secondary", value: null },
      { label: "Replace with this save", kind: "danger", value: "load" },
    ],
  });
  return answer === "load";
}
