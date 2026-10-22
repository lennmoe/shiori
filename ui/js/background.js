import { fileUrl } from "./tauri.js";

export function applyBackground(settings) {
  const root = document.documentElement;
  const image = settings.background_image || "";
  root.classList.toggle("has-bg", !!image);
  root.style.setProperty("--app-bg-image", image ? `url("${fileUrl(image)}")` : "none");
  previewBackground(settings);
}

export function previewBackground(settings) {
  const root = document.documentElement.style;
  root.setProperty("--app-bg-blur", `${settings.background_blur ?? 8}px`);
  root.setProperty("--app-bg-dim", (settings.background_dim ?? 55) / 100);
  root.setProperty("--app-panels", `${settings.background_panels ?? 80}%`);
}
