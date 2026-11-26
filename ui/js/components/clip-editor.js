import { el } from "../dom.js";
import { fileUrl, invoke, listen } from "../tauri.js";
import { show } from "../router.js";
import { ask, call, toast } from "../ui.js";
import { t } from "../i18n.js";

const RATIOS = [
  ["free", "Free", null],
  ["16:9", "16:9", 16 / 9],
  ["1:1", "1:1", 1],
  ["9:16", "9:16", 9 / 16],
  ["4:3", "4:3", 4 / 3],
];
const WIDTHS = [0, 1920, 1280, 960, 720, 480, 360];
const MIN_LENGTH = 0.3;
const MAX_ANIMATED = 60;
const MIN_CROP = 0.04;

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

function clock(seconds) {
  const safe = Math.max(0, seconds || 0);
  const minutes = Math.floor(safe / 60);
  const rest = safe - minutes * 60;
  return `${minutes}:${rest.toFixed(1).padStart(4, "0")}`;
}

function segment(options, value, onPick) {
  const box = el("div", { class: "seg" });
  const paint = (current) => {
    for (const button of box.children) {
      button.classList.toggle("on", button.dataset.v === current);
    }
  };
  for (const [key, label] of options) {
    const button = el("button", { "data-v": key }, label);
    button.onclick = () => {
      paint(key);
      onPick(key);
    };
    box.append(button);
  }
  paint(value);
  return { box, paint };
}

function drag(event, onMove, onEnd) {
  event.preventDefault();
  const move = (e) => onMove(e);
  const up = (e) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    onEnd?.(e);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

export async function openClipEditor({ path, key, name, onDone }) {
  const status = await invoke("recorder_status").catch(() => ({ ready: false }));
  if (!status.ready) {
    const go = await ask(
      "Clip editor",
      t("The clip editor uses the HD recorder (about 115 MB). Download it in Settings → Screenshots first."),
      { yes: "Open settings" },
    );
    if (go) {
      show("settings", { tab: "screenshots" });
    }
    return;
  }

  const state = {
    duration: 0,
    width: 0,
    height: 0,
    start: 0,
    end: 0,
    crop: { x: 0, y: 0, w: 1, h: 1 },
    ratio: "free",
    format: "mp4",
    size: 0,
    fps: 15,
    audio: true,
    busy: false,
  };

  const video = el("video", { src: fileUrl(path), preload: "auto", playsinline: true, disablepictureinpicture: true });
  const cropBox = el(
    "div",
    { class: "ce-crop" },
    ...["nw", "ne", "sw", "se"].map((corner) => el("i", { class: `ce-handle ${corner}`, "data-corner": corner })),
  );
  const frame = el("div", { class: "ce-frame" }, video, cropBox);
  const stage = el("div", { class: "ce-stage" }, frame);

  const range = el("div", { class: "ce-range" });
  const startHandle = el("i", { class: "ce-trim start", title: t("Start") });
  const endHandle = el("i", { class: "ce-trim end", title: t("End") });
  const playhead = el("i", { class: "ce-playhead" });
  const track = el("div", { class: "ce-track" }, range, startHandle, endHandle, playhead);
  const playButton = el("button", { class: "btn secondary ce-play", title: t("Play / pause (Space)") }, "▶");
  const timeLabel = el("span", { class: "ce-time" });
  const timeline = el(
    "div",
    { class: "ce-timeline" },
    playButton,
    track,
    timeLabel,
  );

  const info = el("div", { class: "ce-info muted small-text" });
  const note = el("div", { class: "hint-text ce-note" });
  const progressFill = el("i");
  const progressText = el("span", { class: "muted small-text" });
  const progress = el("div", { class: "ce-progress", hidden: true }, el("div", { class: "bar" }, progressFill), progressText);
  const exportButton = el("button", { class: "btn" }, "Export");
  const cancelButton = el("button", { class: "btn secondary", hidden: true }, "Cancel");

  const ratioSeg = segment(
    RATIOS.map(([value, label]) => [value, t(label)]),
    state.ratio,
    (value) => {
      state.ratio = value;
      fitRatio();
      paint();
    },
  );
  const resetCrop = el("button", { class: "link small-text" }, "Reset crop");
  resetCrop.onclick = () => {
    state.ratio = "free";
    ratioSeg.paint("free");
    state.crop = { x: 0, y: 0, w: 1, h: 1 };
    paint();
  };

  const fpsField = el("div", { class: "ce-field" });
  const audioField = el("label", { class: "ce-check" });
  const audioBox = el("input", { type: "checkbox", checked: true });
  audioBox.onchange = () => {
    state.audio = audioBox.checked;
  };
  audioField.append(audioBox, el("span", {}, t("Keep the sound")));

  const fpsSeg = segment(
    [
      ["10", "10 fps"],
      ["15", "15 fps"],
      ["24", "24 fps"],
    ],
    String(state.fps),
    (value) => {
      state.fps = Number(value);
      paint();
    },
  );
  fpsField.append(el("span", { class: "ce-label" }, t("Smoothness")), fpsSeg.box);

  const formatSeg = segment(
    [
      ["mp4", "MP4"],
      ["gif", "GIF"],
      ["webp", "WebP"],
    ],
    state.format,
    (value) => {
      state.format = value;
      if (value !== "mp4" && !state.size) {
        state.size = 480;
        sizeSelect.value = "480";
      }
      paint();
    },
  );

  const sizeSelect = el("select", {});
  sizeSelect.onchange = () => {
    state.size = Number(sizeSelect.value);
    paint();
  };

  const side = el(
    "aside",
    { class: "ce-side" },
    el("div", { class: "ce-field" }, el("span", { class: "ce-label" }, t("Crop")), ratioSeg.box, resetCrop),
    el("div", { class: "ce-field" }, el("span", { class: "ce-label" }, t("Format")), formatSeg.box),
    el("div", { class: "ce-field" }, el("span", { class: "ce-label" }, t("Width")), sizeSelect),
    fpsField,
    audioField,
    info,
    note,
    el("div", { class: "ce-grow" }),
    el("div", { class: "hint-text" }, "Drag the white box to crop. Space plays, I and O set the start and the end."),
    progress,
    el("div", { class: "row ce-actions" }, cancelButton, exportButton),
  );

  const closeButton = el("button", { class: "btn secondary small", title: t("Close (Esc)") }, "✕");
  const dialog = el(
    "dialog",
    { class: "clip-editor" },
    el(
      "div",
      { class: "ce-shell" },
      el("header", { class: "ce-head" }, el("b", {}, t("Edit clip")), el("span", { class: "muted small-text ce-name" }, name || ""), closeButton),
      el("div", { class: "ce-main" }, el("div", { class: "ce-left" }, stage, timeline), side),
    ),
  );

  let stopProgress = null;
  let frameLoop = 0;
  let resizeObserver = null;

  const cropPixels = () => {
    const { x, y, w, h } = state.crop;
    if (x <= 0.001 && y <= 0.001 && w >= 0.999 && h >= 0.999) {
      return null;
    }
    const px = Math.round(x * state.width);
    const py = Math.round(y * state.height);
    return {
      x: px,
      y: py,
      width: Math.min(state.width - px, Math.round(w * state.width)),
      height: Math.min(state.height - py, Math.round(h * state.height)),
    };
  };

  const outputSize = () => {
    const crop = cropPixels();
    const width = crop ? crop.width : state.width;
    const height = crop ? crop.height : state.height;
    if (!state.size || state.size >= width) {
      return { width, height, scaled: 0 };
    }
    return { width: state.size, height: Math.round((height * state.size) / width), scaled: state.size };
  };

  function fillSizes() {
    const crop = cropPixels();
    const full = crop ? crop.width : state.width;
    const options = WIDTHS.filter((w) => w === 0 || w < full);
    if (!options.includes(state.size)) {
      state.size = state.format === "mp4" ? 0 : options.find((w) => w && w <= 480) || 0;
    }
    sizeSelect.replaceChildren(
      ...options.map((w) =>
        el(
          "option",
          { value: String(w), selected: w === state.size },
          w ? t("{n} px wide", { n: w }) : t("Original ({n} px)", { n: full }),
        ),
      ),
    );
  }

  function fitRatio() {
    const ratio = RATIOS.find(([value]) => value === state.ratio)?.[2];
    if (!ratio || !state.width) {
      return;
    }
    const sourceRatio = state.width / state.height;
    let w = 1;
    let h = sourceRatio / ratio;
    if (h > 1) {
      h = 1;
      w = ratio / sourceRatio;
    }
    const { x, y, w: oldW, h: oldH } = state.crop;
    const cx = x + oldW / 2;
    const cy = y + oldH / 2;
    state.crop = { x: clamp(cx - w / 2, 0, 1 - w), y: clamp(cy - h / 2, 0, 1 - h), w, h };
  }

  function layout() {
    if (!state.width) {
      return;
    }
    const space = 24;
    const scale = Math.min((stage.clientWidth - space) / state.width, (stage.clientHeight - space) / state.height);
    frame.style.width = `${Math.floor(state.width * scale)}px`;
    frame.style.height = `${Math.floor(state.height * scale)}px`;
  }

  function paint() {
    const { x, y, w, h } = state.crop;
    Object.assign(cropBox.style, { left: `${x * 100}%`, top: `${y * 100}%`, width: `${w * 100}%`, height: `${h * 100}%` });
    if (state.duration) {
      const a = (state.start / state.duration) * 100;
      const b = (state.end / state.duration) * 100;
      Object.assign(range.style, { left: `${a}%`, width: `${b - a}%` });
      startHandle.style.left = `${a}%`;
      endHandle.style.left = `${b}%`;
    }
    const length = state.end - state.start;
    timeLabel.textContent = `${clock(state.start)} – ${clock(state.end)} · ${length.toFixed(1)} s`;
    const animated = state.format !== "mp4";
    fpsField.hidden = !animated;
    audioField.hidden = animated;
    fillSizes();
    const size = outputSize();
    info.textContent = `${size.width} × ${size.height} · ${length.toFixed(1)} s`;
    const tooLong = animated && length > MAX_ANIMATED;
    note.textContent = tooLong
      ? t("GIF and WebP can be 60 seconds at most. Trim the clip first.")
      : animated && length > 15
        ? t("Long GIFs get big. WebP is much smaller for the same quality.")
        : "";
    note.classList.toggle("error-text", tooLong);
    exportButton.disabled = state.busy || tooLong || length < MIN_LENGTH;
  }

  function tick() {
    if (state.duration) {
      if (!video.paused && video.currentTime >= state.end) {
        video.currentTime = state.start;
      }
      playhead.style.left = `${(video.currentTime / state.duration) * 100}%`;
    }
    frameLoop = requestAnimationFrame(tick);
  }

  function togglePlay() {
    if (video.paused) {
      if (video.currentTime < state.start || video.currentTime >= state.end - 0.05) {
        video.currentTime = state.start;
      }
      video.play().catch(() => {});
    } else {
      video.pause();
    }
  }

  video.addEventListener("play", () => (playButton.textContent = "❚❚"));
  video.addEventListener("pause", () => (playButton.textContent = "▶"));
  playButton.onclick = togglePlay;
  video.onclick = togglePlay;

  const timeAt = (clientX) => {
    const box = track.getBoundingClientRect();
    return clamp((clientX - box.left) / box.width, 0, 1) * state.duration;
  };

  track.addEventListener("pointerdown", (event) => {
    if (event.target === startHandle || event.target === endHandle || !state.duration) {
      return;
    }
    const seek = (e) => {
      video.currentTime = clamp(timeAt(e.clientX), state.start, state.end);
    };
    seek(event);
    drag(event, seek);
  });

  const trimDrag = (which) => (event) => {
    if (!state.duration) {
      return;
    }
    video.pause();
    drag(event, (e) => {
      const at = timeAt(e.clientX);
      if (which === "start") {
        state.start = clamp(at, 0, state.end - MIN_LENGTH);
        video.currentTime = state.start;
      } else {
        state.end = clamp(at, state.start + MIN_LENGTH, state.duration);
        video.currentTime = state.end;
      }
      paint();
    });
  };
  startHandle.addEventListener("pointerdown", trimDrag("start"));
  endHandle.addEventListener("pointerdown", trimDrag("end"));

  cropBox.addEventListener("pointerdown", (event) => {
    const corner = event.target.dataset?.corner;
    const box = frame.getBoundingClientRect();
    const begin = { ...state.crop, px: event.clientX, py: event.clientY };
    const ratio = RATIOS.find(([value]) => value === state.ratio)?.[2];
    const frameRatio = state.width / state.height;
    drag(event, (e) => {
      const dx = (e.clientX - begin.px) / box.width;
      const dy = (e.clientY - begin.py) / box.height;
      if (!corner) {
        state.crop = {
          ...state.crop,
          x: clamp(begin.x + dx, 0, 1 - begin.w),
          y: clamp(begin.y + dy, 0, 1 - begin.h),
        };
        paint();
        return;
      }
      const left = corner.includes("w");
      const top = corner.includes("n");
      const anchorX = left ? begin.x + begin.w : begin.x;
      const anchorY = top ? begin.y + begin.h : begin.y;
      let w = clamp(left ? begin.w - dx : begin.w + dx, MIN_CROP, left ? anchorX : 1 - anchorX);
      let h = clamp(top ? begin.h - dy : begin.h + dy, MIN_CROP, top ? anchorY : 1 - anchorY);
      if (ratio) {
        const wanted = ratio / frameRatio;
        if (w / h > wanted) {
          w = h * wanted;
        } else {
          h = w / wanted;
        }
        const maxW = left ? anchorX : 1 - anchorX;
        const maxH = top ? anchorY : 1 - anchorY;
        if (w > maxW) {
          w = maxW;
          h = w / wanted;
        }
        if (h > maxH) {
          h = maxH;
          w = h * wanted;
        }
      }
      state.crop = { x: left ? anchorX - w : anchorX, y: top ? anchorY - h : anchorY, w, h };
      paint();
    });
  });

  function setBusy(busy) {
    state.busy = busy;
    progress.hidden = !busy;
    cancelButton.hidden = !busy;
    for (const control of side.querySelectorAll("button:not(.ce-keep), select, input")) {
      if (control !== cancelButton) {
        control.disabled = busy;
      }
    }
    closeButton.disabled = busy;
    paint();
  }

  exportButton.onclick = async () => {
    video.pause();
    progressFill.style.width = "0%";
    progressText.textContent = t("Exporting… {n}%", { n: 0 });
    setBusy(true);
    stopProgress = await listen("clip-export-progress", ({ payload }) => {
      progressFill.style.width = `${payload.percent}%`;
      progressText.textContent = t("Exporting… {n}%", { n: payload.percent });
    });
    try {
      const output = await call("export_clip", {
        key,
        path,
        edit: {
          start: state.start,
          end: state.end,
          crop: cropPixels(),
          format: state.format,
          width: outputSize().scaled,
          fps: state.fps,
          audio: state.audio,
        },
      });
      toast(t("Saved {name}", { name: output.split(/[\\/]/).pop() }));
      close();
      onDone?.(output);
    } catch {
      setBusy(false);
    } finally {
      stopProgress?.();
      stopProgress = null;
    }
  };
  cancelButton.onclick = () => invoke("cancel_clip_export");

  function onKey(event) {
    if (event.target.matches?.("input, select, textarea")) {
      return;
    }
    if (event.key === " ") {
      event.preventDefault();
      togglePlay();
    } else if (event.key === "i" || event.key === "I") {
      state.start = clamp(video.currentTime, 0, state.end - MIN_LENGTH);
      paint();
    } else if (event.key === "o" || event.key === "O") {
      state.end = clamp(video.currentTime, state.start + MIN_LENGTH, state.duration);
      paint();
    }
  }

  function close() {
    cancelAnimationFrame(frameLoop);
    resizeObserver?.disconnect();
    document.removeEventListener("keydown", onKey);
    video.pause();
    video.removeAttribute("src");
    video.load();
    dialog.close();
    dialog.remove();
  }

  dialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    if (!state.busy) {
      close();
    }
  });
  closeButton.onclick = () => !state.busy && close();

  video.addEventListener(
    "loadedmetadata",
    () => {
      state.duration = video.duration || 0;
      state.width = video.videoWidth;
      state.height = video.videoHeight;
      state.end = state.duration;
      layout();
      paint();
    },
    { once: true },
  );

  document.body.append(dialog);
  dialog.showModal();
  document.addEventListener("keydown", onKey);
  resizeObserver = new ResizeObserver(layout);
  resizeObserver.observe(stage);
  frameLoop = requestAnimationFrame(tick);
  paint();
}
