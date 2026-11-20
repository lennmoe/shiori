import { el } from "../dom.js";
import { call, openDialog, toast } from "../ui.js";
import { plural, t } from "../i18n.js";
import { shortDuration } from "../format.js";

const STATUS = { playing: "Reading", finished: "Finished", stalled: "Stalled", dropped: "Dropped", "": "No status" };

const FILTERS = [
  ["all", "All"],
  ["status", "Status"],
  ["vote", "Ratings"],
  ["time", "Time read"],
  ["add", "New"],
];

const score = (vote) => (vote ? `★ ${Number.isInteger(vote / 10) ? vote / 10 : (vote / 10).toFixed(1)}` : "—");
const day = (at) => (at ? new Date(at * 1000).toLocaleDateString() : "—");

function describe(change) {
  switch (change.field) {
    case "status":
      return ["Status", `${t(STATUS[change.shiori] ?? change.shiori)} → ${t(STATUS[change.vndb] ?? change.vndb)}`];
    case "vote":
      return ["Rating", `${score(change.shiori)} → ${score(change.vndb)}`];
    case "finished":
      return ["Finished on", day(change.vndb)];
    case "time":
      return [
        "Time read",
        t("{shiori} in Shiori, {vndb} on VNDB → +{before} before Shiori", {
          shiori: shortDuration(change.tracked),
          vndb: shortDuration(change.vndb),
          before: shortDuration(change.before),
        }),
      ];
    case "add": {
      const parts = [t(STATUS[change.vndb.status] ?? "")];
      if (change.vndb.vote) parts.push(score(change.vndb.vote));
      if (change.vndb.seconds) parts.push(shortDuration(change.vndb.seconds));
      return ["Add to Library", parts.join(" · ")];
    }
    default:
      return [change.field, ""];
  }
}

function cover(item) {
  if (!item.cover || item.nsfw) {
    return el("span", { class: "pull-cover empty" });
  }
  return el("img", { class: "pull-cover", src: item.cover, alt: "", loading: "lazy" });
}

function render(items, onChange) {
  const rows = [];
  const list = el("div", { class: "pull-list" });
  for (const item of items) {
    const lines = item.changes.map((change) => {
      const [label, text] = describe(change);
      const box = el("input", { type: "checkbox", checked: change.checked });
      box.onchange = () => {
        change.checked = box.checked;
        onChange();
      };
      const line = el(
        "label",
        { class: "pull-line", "data-field": change.field },
        box,
        el("span", { class: "pull-label" }, t(label)),
        el("span", { class: "pull-text" }, text),
      );
      rows.push(line);
      return line;
    });
    list.append(
      el(
        "div",
        { class: "pull-item" },
        cover(item),
        el("div", { class: "pull-body" }, el("b", { class: "pull-title" }, item.title), ...lines),
      ),
    );
  }

  const chips = FILTERS.map(([value, label]) => {
    const chip = el("button", { class: `pull-chip${value === "all" ? " on" : ""}`, type: "button" }, t(label));
    chip.onclick = () => {
      for (const other of chips) other.classList.toggle("on", other === chip);
      for (const row of rows) row.hidden = value !== "all" && row.dataset.field !== value;
      for (const item of list.children) {
        item.hidden = [...item.querySelectorAll(".pull-line")].every((row) => row.hidden);
      }
    };
    return chip;
  });
  return el("div", { class: "pull" }, el("div", { class: "pull-chips" }, chips), list);
}

export async function pullFromVndb(button) {
  const label = button.textContent;
  button.disabled = true;
  button.textContent = t("Reading your VNDB list…");
  let preview;
  try {
    preview = await call("vndb_pull_preview");
  } catch {
    return;
  } finally {
    button.disabled = false;
    button.textContent = label;
  }
  const items = preview?.items ?? [];
  if (!items.length) {
    toast(t("Your VNDB list and Shiori already match."));
    return;
  }

  const picked = () => items.reduce((sum, item) => sum + item.changes.filter((c) => c.checked).length, 0);
  let applyButton = null;
  const refresh = () => {
    if (applyButton) {
      const count = picked();
      applyButton.disabled = !count;
      applyButton.textContent = count ? t("Apply {changes}", { changes: plural(count, "change") }) : t("Apply");
    }
  };
  const body = el(
    "div",
    {},
    el("p", { class: "muted pull-intro" }, t("Nothing changes until you apply. Shiori keeps its own value unless you tick VNDB's.")),
    render(items, refresh),
  );

  const answer = await openDialog({
    title: t("Load from VNDB"),
    body,
    wide: true,
    actions: [
      { label: "Cancel", kind: "secondary", value: null },
      { label: "Apply", value: "apply" },
    ],
    onOpen: () => {
      applyButton = document.querySelector("dialog[open] .dialog-actions .btn:last-child");
      refresh();
    },
  });
  if (answer !== "apply") {
    return;
  }
  const picks = items
    .map((item) => ({ id: item.id, fields: item.changes.filter((c) => c.checked).map((c) => c.field) }))
    .filter((pick) => pick.fields.length);
  const count = await call("vndb_pull_apply", { picks }).catch(() => null);
  if (count != null) {
    toast(count ? t("Updated {vns} from VNDB", { vns: plural(count, "VN") }) : t("Nothing to update"));
  }
}
