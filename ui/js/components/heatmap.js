import { el, svg } from "../dom.js";
import { addDays, exactDuration, isoDay, midnight, shortDate, shortMonth, weekday } from "../format.js";
import { t } from "../i18n.js";

const WEEKS = 53;
const CELL = 11;
const GAP = 3;
const PAD_LEFT = 26;
const PAD_TOP = 16;
const LEVELS = [30, 60, 120].map((minutes) => minutes * 60);
const MAX_NAMES = 3;

function level(seconds) {
  if (!seconds) {
    return 0;
  }
  return 1 + LEVELS.filter((limit) => seconds >= limit).length;
}

function tipText(day, info, today) {
  const date = `${weekday(day)} ${shortDate(day, today)}`;
  if (!info) {
    return `${date}  ·  ${t("nothing read")}`;
  }
  const names = info.names.slice(0, MAX_NAMES).join(", ");
  const more = info.names.length > MAX_NAMES ? ` +${info.names.length - MAX_NAMES}` : "";
  return `${date}  ·  ${exactDuration(info.seconds)}  ·  ${names}${more}`;
}

export function drawHeatmap(container, days) {
  const today = midnight(new Date());
  const thisMonday = addDays(today, -((today.getDay() + 6) % 7));
  const firstMonday = addDays(thisMonday, -7 * (WEEKS - 1));
  const width = PAD_LEFT + WEEKS * (CELL + GAP);
  const height = PAD_TOP + 7 * (CELL + GAP);

  const map = svg("svg", { viewBox: `0 0 ${width} ${height}` });
  const tip = el("div", { class: "chart-tip", hidden: true });

  ["Mon", "Wed", "Fri"].forEach((label, i) => {
    const y = PAD_TOP + i * 2 * (CELL + GAP) + CELL - 2;
    map.append(svg("text", { x: 0, y }, t(label)));
  });

  for (let week = 0; week < WEEKS; week++) {
    const monday = addDays(firstMonday, week * 7);
    const sunday = addDays(monday, 6);
    const x = PAD_LEFT + week * (CELL + GAP);

    if (sunday.getDate() <= 7 && week < WEEKS - 1) {
      map.append(svg("text", { x, y: 10 }, shortMonth(sunday)));
    }

    for (let row = 0; row < 7; row++) {
      const day = addDays(monday, row);
      if (day > today) {
        break;
      }
      const info = days[isoDay(day)];
      const y = PAD_TOP + row * (CELL + GAP);
      const cell = svg("rect", {
        x,
        y,
        width: CELL,
        height: CELL,
        rx: 2,
        class: `level-${level(info?.seconds)}`,
      });
      cell.addEventListener("mouseenter", () => {
        const left = ((x + CELL / 2) / width) * 100;
        const shift = left < 20 ? "0%" : left > 80 ? "-100%" : "-50%";
        tip.textContent = tipText(day, info, today);
        tip.style.left = `${left}%`;
        tip.style.top = `${(y / height) * 100}%`;
        tip.style.transform = `translate(${shift}, calc(-100% - 6px))`;
        tip.hidden = false;
      });
      cell.addEventListener("mouseleave", () => (tip.hidden = true));
      map.append(cell);
    }
  }

  container.replaceChildren(el("div", { class: "heatmap" }, map, tip));
}
