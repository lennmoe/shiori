import { t } from "./i18n.js";

export { plural } from "./i18n.js";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function formatPlaytime(seconds) {
  const minutes = Math.floor(Math.max(0, seconds) / 60);
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
}

export function lengthText(minutes) {
  if (minutes <= 0) {
    return "";
  }
  const hours = minutes / 60;
  if (hours >= 2) {
    return `~${Math.round(hours)}h`;
  }
  return minutes < 60 ? `~${minutes}m` : `~${hours.toFixed(1)}h`;
}

export function readingProgress(seconds, lengthMinutes) {
  if (!(lengthMinutes > 0)) {
    return null;
  }
  return Math.floor(((seconds || 0) / 60 / lengthMinutes) * 100);
}

export function shortDuration(seconds) {
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;

  if (hours && rest) {
    return `${hours}h ${rest}m`;
  }
  return hours ? `${hours}h` : `${rest}m`;
}

export function exactDuration(seconds) {
  return seconds >= 3600 ? formatPlaytime(seconds) : `${Math.floor(seconds / 60)}m`;
}

export function monthName(date) {
  return t(LONG_MONTHS[date.getMonth()]);
}

export function shortMonth(date) {
  return t(MONTHS[date.getMonth()]);
}

export function weekday(date) {
  return t(WEEKDAYS[date.getDay()]);
}

export function dayMonth(date) {
  return t("{day} {month}", { day: date.getDate(), month: shortMonth(date) });
}

export function shortDate(date, today = new Date()) {
  const label = dayMonth(date);
  return date.getFullYear() === today.getFullYear() ? label : `${label} ${date.getFullYear()}`;
}

export function midnight(date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

export function addDays(date, days) {
  const copy = new Date(date);
  copy.setDate(copy.getDate() + days);
  return copy;
}

export function isoDay(date) {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

export function parseDay(text) {
  const [year, month, day] = text.split("-").map(Number);
  return new Date(year, month - 1, day);
}

export function lastPlayed(timestamp) {
  if (!timestamp) {
    return "";
  }
  const day = midnight(new Date(timestamp * 1000));
  const today = midnight(new Date());
  const delta = Math.round((today - day) / 86400000);

  if (delta <= 0) {
    return t("today");
  }
  if (delta === 1) {
    return t("yesterday");
  }
  if (delta < 7) {
    return t("{n} days ago", { n: delta });
  }
  return shortDate(day, today);
}

export function capitalize(text) {
  return text ? text[0].toUpperCase() + text.slice(1) : "";
}

