import { $, el, html } from "../dom.js";
import { invoke, pickFile, pickFolder, pickSaveFile } from "../tauri.js";
import { register, show } from "../router.js";
import { store, subscribe, update } from "../store.js";
import { ask, call, toast, toggle } from "../ui.js";
import { hotkeyInput } from "../components/hotkey-input.js";
import { renderThemeTab } from "../components/theme-tab.js";
import { offerUpdate } from "../components/update.js";
import { discordButton, discordLogo } from "../components/discord-button.js";
import { languagePicker } from "../components/language-picker.js";
import { plural, t } from "../i18n.js";
import { presenceBuilder } from "./presence.js";
import { communityTab, warmCommunity } from "../components/community-tab.js";
import { defaultReleaseMedium } from "../release-filters.js";
import { pullFromVndb } from "../components/vndb-pull.js";
import { previewCloud } from "../components/cloud-preview.js";

const TEMPLATE = `
  <div class="page-head">
    <h2>Settings</h2>
  </div>

  <div class="settings-layout">
    <nav class="settings-nav">
      <input class="settings-search" data-id="search" type="search" placeholder="Search settings…" />
      <div class="settings-tabs" data-id="tabs">
        <button data-tab="general">General</button>
        <button data-tab="library">Library</button>
        <button data-tab="presence">Presence</button>
        <button data-tab="screenshots">Screenshots</button>
        <button data-tab="profile">Profile & backup</button>
        <button data-tab="community">Community</button>
        <button data-tab="theme">Theme</button>
        <button data-tab="detection">Detection</button>
        <button data-tab="advanced">Advanced</button>
      </div>
    </nav>
    <div class="card settings-body" data-id="body"></div>
  </div>

  <div class="settings-footer">
    <span class="muted" data-id="version"></span>
    <div class="row" data-id="footer-actions">
      <button class="btn secondary small" data-id="whats-new">What's new</button>
      <button class="btn secondary small" data-id="check-update">Check for updates</button>
    </div>
  </div>
`;

let root;
let tab = "general";
let query = "";

const OLD_TABS = { data: "profile", blacklist: "detection" };

const part = (id) => $(`[data-id="${id}"]`, root);

function build(container) {
  root = container;
  root.append(html(TEMPLATE));

  for (const button of part("tabs").querySelectorAll("button")) {
    button.onclick = () => {
      tab = button.dataset.tab;
      query = "";
      part("search").value = "";
      const body = part("body");
      body.classList.remove("tab-in");
      clearTimeout(body.tabIn);
      render();
      body.scrollTop = 0;
      void body.offsetWidth;
      body.classList.add("tab-in");
      body.tabIn = setTimeout(() => body.classList.remove("tab-in"), 500);
    };
  }

  let typing = null;
  part("search").addEventListener("input", (event) => {
    clearTimeout(typing);
    typing = setTimeout(() => {
      query = event.target.value.trim().toLowerCase();
      render();
    }, 200);
  });

  buildFooter();
  setTimeout(warmCommunity, 4000);

  subscribe("settings", () => {
    const typing =
      root.contains(document.activeElement) && document.activeElement.matches("input:not([type=checkbox]), textarea");
    if (root.classList.contains("active") && !typing) {
      render();
    }
  });
}

function showPage(params = {}) {
  if (params.tab) {
    tab = OLD_TABS[params.tab] || params.tab;
    query = "";
    part("search").value = "";
  }
  render();
  if (params.focus) {
    $(`[data-section="${params.focus}"]`, root)?.scrollIntoView({ block: "start" });
  }
}

function buildFooter() {
  part("version").textContent = `Shiori RPC ${store.version}`;

  const check = part("check-update");
  check.onclick = async () => {
    check.disabled = true;
    check.textContent = t("Checking…");
    await offerUpdate({ quiet: false });
    check.disabled = false;
    check.textContent = t("Check for updates");
  };

  part("whats-new").onclick = () => show("whatsnew", { recent: 5 });
  part("whats-new").before(discordButton({ small: true }));
}

async function save(changes) {
  update("settings", { ...store.settings, ...changes });
  await call("update_settings", { changes });
}

async function quietSave(changes) {
  Object.assign(store.settings, changes);
  await call("update_settings", { changes });
}

const TABS = {
  general: () => generalTab(),
  library: () => libraryTab(),
  presence: () => presenceTab(),
  screenshots: () => screenshotsSection(),
  profile: () => dataSection(),
  community: () => communityTab(),
  theme: () => {
    const box = el("div");
    renderThemeTab(box, save);
    return [box];
  },
  detection: () => [section("Never detect"), ...blacklistTab(), section("Title rules"), ...rulesTab()],
  advanced: () => advancedTab(),
};

function render() {
  const body = part("body");
  for (const button of part("tabs").querySelectorAll("button")) {
    button.classList.toggle("on", !query && button.dataset.tab === tab);
  }
  if (!query) {
    body.replaceChildren(...(TABS[tab] || TABS.general)());
    return;
  }
  const groups = [];
  for (const button of part("tabs").querySelectorAll("button")) {
    const group = el(
      "div",
      { class: "settings-group" },
      el("div", { class: "settings-group-title" }, button.textContent),
      ...TABS[button.dataset.tab](),
    );
    if (filterGroup(group)) {
      groups.push(group);
    }
  }
  body.replaceChildren(...(groups.length ? groups : [el("div", { class: "muted" }, "No setting matches.")]));
}

function matches(node) {
  const words = [node.textContent];
  for (const field of node.querySelectorAll?.("[placeholder], [title]") || []) {
    words.push(field.getAttribute("placeholder") || "", field.getAttribute("title") || "");
  }
  return words.join(" ").toLowerCase().includes(query);
}

function filterGroup(group) {
  const [title, ...items] = group.children;
  if (matches(title)) {
    return true;
  }
  const blocks = [];
  for (const item of items) {
    if (item.classList.contains("section-title") || !blocks.length) {
      blocks.push({ title: item.classList.contains("section-title") ? item : null, items: [] });
    }
    if (!item.classList.contains("section-title")) {
      blocks.at(-1).items.push(item);
    }
  }
  let any = false;
  for (const block of blocks) {
    const all = block.title && matches(block.title);
    let shown = false;
    for (const item of block.items) {
      const keep = all || matches(item);
      item.hidden = !keep;
      shown ||= keep;
    }
    if (block.title) {
      block.title.hidden = !shown;
    }
    any ||= shown;
  }
  return any;
}

const section = (title, id = "") => el("div", { class: "section-title", "data-section": id }, title);

const hint = (text) => el("div", { class: "hint-text" }, text);

function setting(key, label, hintText = "") {
  return toggle(label, store.settings[key], (on) => save({ [key]: on }), hintText);
}

function numberField(key, label, { min, max, fallback }) {
  const input = el("input", { type: "number", min, max, value: store.settings[key] ?? fallback });
  input.onchange = () => {
    const value = Math.min(max, Math.max(min, Math.round(Number(input.value) || fallback)));
    input.value = value;
    save({ [key]: value });
  };
  return el("div", { class: "field" }, el("span", {}, label), input);
}

function textField(key, { placeholder = "", type = "text" } = {}) {
  const input = el("input", { type, class: "wide", placeholder, value: store.settings[key] || "" });
  input.onchange = () => save({ [key]: input.value.trim() });
  return input;
}

function pathRow(key, { title, filters, folder = false, placeholder = "Not set" }) {
  const input = textField(key, { placeholder });

  const browse = el("button", { class: "btn secondary" }, "Browse…");
  browse.onclick = async () => {
    const options = { title, filters, defaultPath: input.value || undefined };
    const path = folder ? await pickFolder(options) : await pickFile(options);
    if (path) {
      input.value = path;
      save({ [key]: path });
    }
  };

  return el("div", { class: "row" }, input, browse);
}

function testedField(input, test) {
  const result = el("div", { class: "hint-text" });
  const button = el("button", { class: "btn secondary" }, "Test");

  button.onclick = async () => {
    result.className = "hint-text";
    result.textContent = t("Testing…");
    try {
      const message = await test(input.value.trim());
      result.className = "hint-text ok-text";
      result.textContent = t(message);
    } catch (error) {
      result.className = "hint-text error-text";
      result.textContent = t("Failed: {error}", { error: t(String(error)) });
    }
  };

  return [el("div", { class: "row" }, input, button), result];
}

function languageSection() {
  const picker = languagePicker(async (language) => {
    await save({ language });
    location.reload();
  });
  return [section("Language"), el("div", { class: "row" }, picker)];
}

function generalTab() {
  return [...languageSection(), ...appSection()];
}

function libraryTab() {
  return [...coversSection(), ...vndbSection(), ...walkthroughSection(), ...localeSection()];
}

function advancedTab() {
  return [
    hint("Only needed if something doesn't work as expected."),
    section("Discord"),
    ...discordSection(),
    numberField("update_min_interval", "Min. seconds between presence updates", {
      min: 1,
      max: 300,
      fallback: 5,
    }),
    el("div", { class: "field-label" }, "Fallback Discord asset key"),
    textField("default_asset_key"),
    hint(
      "The image Discord shows when the cover can't be: a local file, an NSFW cover, or no cover.",
    ),
  ];
}

function discordSection() {
  const clientId = textField("discord_client_id");
  const test = async (id) => {
    await invoke("test_discord", { clientId: id });
    return "Connected — this ID works.";
  };

  return [
    el("div", { class: "field-label" }, "Discord Application ID"),
    ...testedField(clientId, test),
    hint(
      "Create one at discord.com/developers → New Application, then paste its Application ID. " +
        "No bot or OAuth needed.",
    ),
  ];
}

function presenceLanguageField() {
  const picker = languagePicker((code) => save({ presence_language: code }), {
    current: store.settings.presence_language || "app",
    first: { code: "app", name: "Same as the app", flag: "__" },
    right: true,
  });
  return el("div", { class: "field" }, el("span", {}, "Language on Discord"), picker);
}

function titleLanguageField() {
  const select = el(
    "select",
    {},
    el("option", { value: "romanized" }, t("Romanized")),
    el("option", { value: "original" }, t("Original")),
    el("option", { value: "english" }, t("English")),
  );
  select.value = store.settings.title_language || "romanized";
  select.onchange = () => save({ title_language: select.value });
  return el("div", { class: "field" }, el("span", {}, "Title on Discord"), select);
}

function presenceTab() {
  const custom = !!store.settings.presence_custom?.enabled;
  return [
    ...presenceSection(custom),
    el("div", { class: "section-title" }, "Your own layout", " ", el("span", { class: "labs-tag" }, "beta")),
    presenceBuilder(),
  ];
}

function presenceSection(custom) {
  return [
    section("Presence"),
    ...(custom ? [hint("Your own layout is on: it replaces the lines set below.")] : []),
    presenceLanguageField(),
    titleLanguageField(),
    hint("The original and English titles come from VNDB. VNs without one keep the romanized title."),
    el(
      "div",
      { class: custom ? "locked" : "", title: custom ? "Set by your own layout" : "" },
      setting("show_section", "Show where you are in the story", "The “Reading・Chapter 1” line."),
      setting("show_total_read", "Show total time read", "The “Total read: 3h 40m” line."),
      setting(
        "show_progress",
        "Show how far you are",
        "Adds “· 78%” to the time read, compared with the average length on VNDB.",
      ),
      setting("show_elapsed", "Show elapsed time"),
      setting("show_vndb_button", "Add a “View on VNDB” button"),
    ),
    setting("clear_on_close", "Clear presence when the VN closes"),
    setting(
      "idle_when_unfocused",
      "Go idle when the VN isn't the active window",
      "Stops the elapsed time and the time read until you go back to the game.",
    ),
    store.settings.idle_when_unfocused &&
      numberField("idle_seconds", "Seconds in the background before going idle", {
        min: 0,
        max: 3600,
        fallback: 60,
      }),
    store.settings.idle_when_unfocused && idlePresence(),
  ].filter(Boolean);
}

function idlePresence() {
  const select = el(
    "select",
    {},
    ...[
      ["idle", "Show “Idle” instead of “Reading”"],
      ["hide", "Clear my Discord status"],
    ].map(([value, label]) =>
      el("option", { value, selected: value === (store.settings.idle_presence || "idle") }, t(label)),
    ),
  );
  select.onchange = () => save({ idle_presence: select.value });
  return el("div", { class: "field" }, el("span", {}, "While idle"), select);
}

function coversSection() {
  const medium = el(
    "select",
    { "aria-label": "Default release medium" },
    el("option", { value: "" }, "Any"),
    el("option", { value: "package" }, "Package"),
    el("option", { value: "download" }, "Download"),
  );
  medium.value = defaultReleaseMedium(store.settings.vndb_release_medium);
  medium.onchange = () => save({ vndb_release_medium: medium.value });
  return [
    section("Covers & matching"),
    setting(
      "allow_nsfw_covers",
      "Allow NSFW-flagged covers",
      "Off: flagged covers are blurred here and replaced by the fallback image on Discord.",
    ),
    setting(
      "use_steam_names",
      "Use Steam library names",
      "Gives much better VNDB matches for games installed through Steam.",
    ),
    el("div", { class: "field" }, el("span", {}, "Default release medium"), medium),
    hint("The initial medium filter when choosing a VNDB release."),
  ];
}

function vndbSection() {
  const token = textField("vndb_token", { type: "password" });
  const test = async (value) => {
    if (!value) {
      throw "Paste a token first.";
    }
    const name = await invoke("check_vndb_token", { token: value });
    return t("Connected as {name}.", { name });
  };

  return [
    section("VNDB list", "vndb"),
    el("div", { class: "field-label" }, "Personal token"),
    ...testedField(token, test),
    hint(
      "vndb.org → your profile → Edit → Applications → New token, with access to your list and " +
        "permission to edit it.",
    ),
    setting(
      "vndb_sync",
      "Sync reading status to my VNDB list",
      "Statuses set in the Library are sent to VNDB, and a VN you start reading leaves your " +
        "wishlist. Only VNs whose VNDB entry you picked or confirmed are synced.",
    ),
    store.settings.vndb_sync &&
      setting(
        "vndb_mark_playing",
        "Mark VNs as Playing on VNDB when I start them",
        "Only when the VN has no status on your list yet.",
      ),
    ...(store.settings.vndb_token ? pullSection() : []),
  ].filter(Boolean);
}

function pullSection() {
  const button = el("button", { class: "btn secondary" }, "Load from VNDB…");
  button.onclick = () => pullFromVndb(button);
  return [
    el("div", { class: "row" }, button),
    hint("Compare your VNDB list with Shiori: statuses, ratings, finish dates, time read and VNs that are missing. You pick what to take, nothing changes before that."),
  ];
}

function screenshotsSection() {
  const key = hotkeyInput(store.settings.screenshot_hotkey, (value) =>
    save({ screenshot_hotkey: value }),
  );

  const volume = el("input", {
    type: "range",
    min: 0,
    max: 100,
    step: 5,
    value: store.settings.screenshot_volume ?? 30,
  });
  const volumeLabel = el("span", { class: "muted", style: { width: "44px", textAlign: "right" } });
  const showVolume = () =>
    (volumeLabel.textContent = Number(volume.value) ? `${volume.value}%` : "Off");
  volume.oninput = showVolume;
  volume.onchange = () => save({ screenshot_volume: Number(volume.value) });
  showVolume();

  const clips = clipsSection();

  const listen = el("button", { class: "btn secondary small", title: "Listen" }, "▶");
  listen.onclick = () => invoke("play_shutter", { volume: Number(volume.value) });

  return [
    el(
      "div",
      { class: "backup-card shots-card" },
      el("b", { class: "backup-card-title" }, "Screenshots"),
      el(
        "div",
        { class: "shots-grid" },
        el(
          "div",
          { class: "shots-item" },
          el("div", { class: "field" }, el("span", {}, "Capture key"), key),
          hint(
            "Click, then press the key or combination (Esc cancels, Backspace turns it off). It only works " +
              "while the game's window is in front, so other programs keep the key.",
          ),
        ),
        el(
          "div",
          { class: "shots-item" },
          el(
            "div",
            { class: "field" },
            el("span", {}, "Capture sound"),
            el("div", { class: "row" }, volume, volumeLabel, listen),
          ),
        ),
      ),
      el(
        "div",
        { class: "shots-folder" },
        el("div", { class: "field-label" }, "Folder"),
        pathRow("screenshot_dir", {
          title: "Screenshot folder",
          folder: true,
          placeholder: store.defaultScreenshotRoot,
        }),
        hint("Each VN gets its own folder in there. Screenshots already taken aren't moved."),
      ),
    ),
    ...clips,
  ];
}

function clipsSection() {
  const choice = (setting, current, options) => {
    const select = el(
      "select",
      {},
      ...options.map(([value, label]) =>
        el("option", { value, selected: value === current }, t(label, { n: value })),
      ),
    );
    select.onchange = () => save({ [setting]: Number(select.value) });
    return select;
  };
  const lengths = [
    [30, "{n} seconds"],
    [60, "1 minute"],
    [120, "2 minutes"],
    [300, "5 minutes"],
  ];
  const field = (label, input) => el("div", { class: "field" }, el("span", {}, label), input);

  const group = (title, on, setting, hintText, fields, extra = {}) => {
    const body = el("div", { class: "clip-group-body" }, ...fields);
    body.hidden = !on;
    const head = toggle(
      title,
      on,
      (value) => {
        body.hidden = !value;
        save({ [setting]: value, ...(value ? extra : {}) });
      },
      hintText,
    );
    return el("div", { class: "clip-group" }, head, body);
  };

  const keyGroup = group(
    "Record with a key",
    store.settings.clip_key_enabled !== false,
    "clip_key_enabled",
    "Press the key while reading to start filming the game's window, and again to stop.",
    [
      field("Clip key", hotkeyInput(store.settings.clip_hotkey, (value) => save({ clip_hotkey: value }))),
      field(
        "Longest clip",
        choice("clip_max_seconds", store.settings.clip_max_seconds ?? 60, [
          ...lengths,
          [600, "10 minutes"],
          [1800, "30 minutes"],
          [0, "No limit"],
        ]),
      ),
    ],
  );

  const replaySeconds = store.settings.clip_replay_seconds || 0;
  const replayGroup = group(
    "Instant replay",
    store.settings.clip_replay_enabled ?? replaySeconds > 0,
    "clip_replay_enabled",
    "Shiori keeps the last moments in memory, so you can save something after it happened. Uses a bit " +
      "of your graphics card while it's on.",
    [
      field("How much to keep", choice("clip_replay_seconds", replaySeconds || 60, lengths)),
      field(
        "Save key",
        hotkeyInput(store.settings.clip_replay_hotkey, (value) => save({ clip_replay_hotkey: value })),
      ),
    ],
    replaySeconds ? {} : { clip_replay_seconds: 60 },
  );

  const quality = el(
    "select",
    {},
    ...[
      ["normal", "Normal (smaller files)"],
      ["high", "High"],
      ["best", "Best (big files)"],
    ].map(([value, label]) =>
      el("option", { value, selected: value === (store.settings.clip_quality || "high") }, t(label)),
    ),
  );
  quality.onchange = () => save({ clip_quality: quality.value });

  const videoGroup = el(
    "div",
    { class: "clip-group" },
    el("div", { class: "clip-group-title" }, "Video"),
    el(
      "div",
      { class: "clip-group-body" },
      field("Smoothness", choice("clip_fps", store.settings.clip_fps ?? 30, [
        [30, "30 fps"],
        [60, "60 fps"],
      ])),
      field("Quality", quality),
      toggle(
        "Record the game's sound",
        store.settings.clip_audio !== false,
        (on) => save({ clip_audio: on }),
        "Only the VN's sound: not Discord or your music.",
      ),
    ),
  );

  const options = el("div", { class: "clip-groups" }, keyGroup, replayGroup, videoGroup);
  options.hidden = store.settings.clip_enabled === false;

  return [
    el(
      "div",
      { class: "backup-card shots-card" },
      el("b", { class: "backup-card-title" }, "Clips"),
      toggle(
        "Turn on clips",
        store.settings.clip_enabled !== false,
        (on) => {
          options.hidden = !on;
          save({ clip_enabled: on });
        },
        "Off: the clip keys stay free for other apps and nothing is recorded. Clips are saved as MP4 next " +
          "to your screenshots.",
      ),
      options,
    ),
  ];
}

function walkthroughSection() {
  const options = el(
    "div",
    { class: "walkthrough-options" },
    el(
      "div",
      { class: "field" },
      el("span", {}, "Walkthrough key"),
      hotkeyInput(store.settings.walkthrough_hotkey, (value) => save({ walkthrough_hotkey: value })),
    ),
    hint("Shows or hides the walkthrough over the game. It only works while the game's window is in front."),
    toggle(
      "Translate to English",
      store.settings.walkthrough_english !== false,
      (on) => save({ walkthrough_english: on }),
      "Google translates it the first time you open a walkthrough. Off: it stays in Japanese.",
    ),
    toggle(
      "Show spoilers",
      !!store.settings.walkthrough_spoilers,
      (on) => save({ walkthrough_spoilers: on }),
      "Off: route names and endings stay blurred until you click them.",
    ),
  );
  options.hidden = store.settings.walkthrough_enabled === false;
  return [
    el("div", { class: "section-title" }, "Walkthrough", " ", el("span", { class: "labs-tag" }, "beta")),
    toggle(
      "Turn on walkthroughs",
      store.settings.walkthrough_enabled !== false,
      (on) => {
        options.hidden = !on;
        save({ walkthrough_enabled: on });
      },
      "Off: no walkthrough on VN pages, no window over the game, and the walkthrough key stays free.",
    ),
    options,
  ];
}

function localeSection() {
  const exe = (name) => [{ name, extensions: ["exe"] }];
  return [
    section("Japanese locale"),
    hint(
      "For VNs that show garbled text or won't start: pick “Launch with” on a game's page in the " +
        "Library, and Play starts it through one of these.",
    ),
    el("div", { class: "field-label" }, "Locale Emulator  (LEProc.exe)"),
    pathRow("locale_emulator_path", { title: "Locate LEProc.exe", filters: exe("LEProc.exe") }),
    el("div", { class: "field-label" }, "NTLEA  (ntleas.exe)"),
    pathRow("ntlea_path", { title: "Locate ntleas.exe", filters: exe("ntleas.exe") }),
  ];
}

function appSection() {
  const autostart = toggle("Launch when Windows starts", store.autostart, async (on) => {
    await call("set_autostart", { enabled: on });
    store.autostart = on;
  });

  return [
    section("App"),
    toggle(
      "Top bar",
      store.settings.labs_topbar !== false,
      (on) => save({ labs_topbar: on }),
      "Shows the VN you're reading, the search, the status and Pause at the top.",
    ),
    setting("start_minimized", "Start minimized to the tray"),
    autostart,
    setting(
      "check_updates",
      "Check for updates at launch",
      t("Version {version}. New GitHub releases are offered when the app starts.", {
        version: store.version,
      }),
    ),
  ];
}

function dataSection() {
  const exportButton = el("button", { class: "btn secondary", onclick: exportData }, "Export…");
  const importButton = el("button", { class: "btn secondary", onclick: importData }, "Import…");

  const file = el(
    "div",
    { class: "backup-card" },
    el("b", { class: "backup-card-title" }, "Backup file"),
    el(
      "span",
      { class: "muted backup-card-text" },
      "Settings, the Library (time read, history, statuses) and custom covers in one file, to move to " +
        "another PC or keep a copy. It includes your VNDB token: don't share it.",
    ),
    el("div", { class: "row backup-card-actions" }, exportButton, importButton),
  );

  return [cloudBox(file)];
}

const PRIVACY_URL = "https://shiori.maia.moe/privacy";
const CLOUD_FRESH = 60000;
let cloudStatus = null;
let cloudChecked = 0;

function cloudDate(ms) {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function cloudSize(bytes) {
  if (bytes < 1024 * 1024) {
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function cloudBox(file) {
  const box = el("div", { class: "cloud-box" });
  const privacy = () =>
    el("button", { class: "link", onclick: () => call("open_url", { url: PRIVACY_URL }) }, "What's stored");

  const show = (...children) => box.replaceChildren(...children);

  const signedOut = () =>
    show(
      el(
        "div",
        { class: "cloud-card cloud-empty" },
        el(
          "div",
          { class: "cloud-pitch" },
          el("b", {}, "Keep your Library safe online"),
          el(
            "span",
            { class: "muted" },
            "Save your settings and Library online, then load them on another PC. You sign in with Discord: " +
              "Shiori only gets your name and avatar.",
          ),
        ),
        el(
          "div",
          { class: "row" },
          el("button", { class: "btn discord-btn", onclick: signIn }, discordLogo(), "Sign in with Discord"),
          privacy(),
        ),
      ),
      file,
    );

  const failed = () =>
    show(
      el("div", { class: "hint-text" }, "Couldn't reach the cloud. Check your connection."),
      el("button", { class: "btn secondary small", onclick: () => refresh({ loud: true }) }, "Try again"),
      file,
    );

  const signedIn = (status) => {
    const backup = status.backup;
    const saved = backup?.updated
      ? el(
          "span",
          { class: "cloud-saved" },
          el("span", { class: "cloud-dot ok" }),
          t("Last cloud save: {date}", { date: cloudDate(backup.updated) }),
          backup.size ? el("span", { class: "muted" }, ` · ${cloudSize(backup.size)}`) : null,
        )
      : el("span", { class: "cloud-saved" }, el("span", { class: "cloud-dot" }), t("Nothing saved in the cloud yet."));
    const saveButton = el("button", { class: "btn" }, "Save to cloud");
    saveButton.onclick = async () => {
      saveButton.disabled = true;
      const result = await call("cloud_save");
      saveButton.disabled = false;
      if (result) {
        toast("Saved to the cloud");
        refresh();
      }
    };
    const loadButton = el("button", { class: "btn secondary", disabled: !backup }, "Load from cloud");
    loadButton.onclick = () => loadCloud(status, loadButton);
    const avatar = status.avatar_url
      ? el("img", { class: "cloud-avatar", src: status.avatar_url, alt: "" })
      : el("span", { class: "cloud-avatar cloud-avatar-empty" }, discordLogo());
    show(
      el(
        "div",
        { class: "cloud-account" },
        avatar,
        el(
          "div",
          { class: "cloud-who" },
          el("b", {}, status.name || "Discord"),
          el("span", { class: "muted cloud-via" }, discordLogo(), "Discord"),
        ),
        privacy(),
        el("button", { class: "btn secondary small", onclick: signOut }, "Sign out"),
      ),
      el(
        "div",
        { class: "backup-grid" },
        el(
          "div",
          { class: "backup-card" },
          el("b", { class: "backup-card-title" }, "Cloud backup"),
          saved,
          el("div", { class: "row backup-card-actions" }, saveButton, loadButton),
          el(
            "div",
            { class: "backup-card-toggles" },
            toggle(
              "Save to the cloud when Shiori closes",
              !!store.settings.sync_auto_save,
              (on) => quietSave({ sync_auto_save: on }),
              "Only when you quit Shiori RPC, not when you close its window.",
            ),
            toggle(
              "Include my VNDB token",
              !!store.settings.sync_vndb_token,
              (on) => quietSave({ sync_vndb_token: on }),
              "So you don't have to paste it again on another PC. Off: it stays on this PC only.",
            ),
          ),
        ),
        file,
      ),
      el(
        "div",
        { class: "cloud-danger" },
        el("button", { class: "link danger-text", onclick: deleteCloud }, "Delete my cloud data"),
      ),
    );
  };

  const draw = (status) => (status.signed_in ? signedIn(status) : signedOut());

  async function refresh({ loud = false } = {}) {
    if (loud || !cloudStatus) {
      show(el("div", { class: "hint-text" }, "Checking your cloud backup…"), file);
    }
    try {
      const status = await invoke("cloud_status");
      const changed = JSON.stringify(status) !== JSON.stringify(cloudStatus);
      cloudStatus = status;
      cloudChecked = Date.now();
      if (changed || loud || !box.firstChild || box.querySelector(".hint-text")) {
        draw(status);
      }
    } catch {
      if (!cloudStatus) {
        failed();
      }
    }
  }

  if (cloudStatus) {
    draw(cloudStatus);
  }
  if (!cloudStatus || Date.now() - cloudChecked > CLOUD_FRESH) {
    refresh();
  }

  async function signIn() {
    show(el("div", { class: "hint-text" }, "Finish signing in in your browser, then come back here."), file);
    try {
      const status = await invoke("cloud_login");
      cloudStatus = status;
      cloudChecked = Date.now();
      draw(status);
      toast("Signed in");
    } catch (error) {
      toast(typeof error === "string" ? error : "Couldn't sign in", { error: true });
      refresh();
    }
  }

  async function signOut() {
    await call("cloud_logout");
    store.settings.sync_auto_save = false;
    cloudStatus = null;
    refresh({ loud: true });
  }

  async function deleteCloud() {
    const sure = await ask(
      "Delete my cloud data",
      "This removes your cloud backup and signs you out on every PC. Your data on this PC stays.",
      { yes: "Delete", danger: true },
    );
    if (sure) {
      await call("cloud_delete");
      toast("Your cloud data was deleted");
      cloudStatus = null;
      refresh({ loud: true });
    }
  }

  async function loadCloud(status, button) {
    const folder = await invoke("backup_folder");
    const sure = await previewCloud(button, { date: cloudDate(status.backup.updated), folder });
    if (!sure) {
      return;
    }
    const count = await call("cloud_load");
    if (count !== undefined && count !== null) {
      toast(t("Loaded {vns} and your settings", { vns: plural(count, "VN") }));
      refresh();
    }
  }

  return box;
}


async function exportData() {
  const path = await pickSaveFile({
    title: "Export data",
    defaultPath: await invoke("backup_default_name"),
    filters: [{ name: "Shiori RPC backup", extensions: ["zip"] }],
  });
  if (!path) {
    return;
  }
  const count = await call("export_data", { path });
  toast(t("Saved {vns} and your settings", { vns: plural(count, "VN") }));
}

async function importData() {
  const path = await pickFile({
    title: "Import data",
    filters: [{ name: "Shiori RPC backup", extensions: ["zip"] }],
  });
  if (!path) {
    return;
  }

  const folder = await invoke("backup_folder");
  const sure = await ask(
    "Import data",
    t(
      "Replace your settings and Library with the ones in this file? What you have now is saved first in {folder}.",
      { folder },
    ),
    { yes: "Replace", danger: true },
  );
  if (!sure) {
    return;
  }

  const count = await call("import_data", { path });
  toast(t("Loaded {vns} and the settings", { vns: plural(count, "VN") }));
}

function checkPattern(pattern) {
  new RegExp(pattern.replace(/\(\?P</g, "(?<"), "iu");
}

function rulesTab() {
  const lines = (store.settings.title_rules || []).map((rule) => JSON.stringify(rule));
  const text = el("textarea", { spellcheck: "false" });
  text.value = lines.join("\n");

  const error = el("div", { class: "hint-text error-text" });
  const saveButton = el("button", { class: "btn" }, "Save rules");

  saveButton.onclick = async () => {
    const rules = [];
    const all = text.value.split("\n");
    for (const [i, raw] of all.entries()) {
      const line = raw.trim();
      if (!line) {
        continue;
      }
      try {
        const rule = JSON.parse(line);
        if (typeof rule !== "object" || typeof rule.pattern !== "string") {
          throw new Error('each rule must be an object with a "pattern" string');
        }
        checkPattern(rule.pattern);
        rules.push(rule);
      } catch (problem) {
        error.textContent = t("Line {n}: {problem}", { n: i + 1, problem: t(problem.message) });
        return;
      }
    }
    error.textContent = "";
    await save({ title_rules: rules });
    toast("Title rules saved");
  };

  const example =
    '{"name": "part", "pattern": "part\\\\s*(?P<n>\\\\d+)", "label": "Part {n}", "section_type": "chapter"}';

  return [
    hint(
      "Extra rules run before the built-in ones. One JSON object per line; the first rule whose pattern " +
        "matches the window title wins.",
    ),
    el(
      "div",
      { class: "process-name", style: { display: "block", margin: "6px 0 10px" } },
      example,
    ),
    text,
    error,
    el("div", { class: "row end" }, saveButton),
  ];
}

function blacklistTab() {
  const list = store.settings.blacklist_exe || [];

  const chips = list.length
    ? list.map((exe) =>
        el(
          "span",
          { class: "chip" },
          exe,
          el(
            "button",
            {
              title: "Detect it again",
              onclick: () => save({ blacklist_exe: list.filter((x) => x !== exe) }),
            },
            "×",
          ),
        ),
      )
    : [el("span", { class: "muted" }, "Nothing yet.")];

  const input = el("input", { type: "text", class: "wide", placeholder: "osu!.exe" });
  const addName = (value) => {
    let name = value.trim().split(/[\\/]/).pop();
    if (!name) {
      return;
    }
    if (!name.toLowerCase().endsWith(".exe")) {
      name += ".exe";
    }
    if (!list.some((x) => x.toLowerCase() === name.toLowerCase())) {
      save({ blacklist_exe: [...list, name] });
    }
  };
  const add = () => {
    addName(input.value);
    input.value = "";
  };

  const picker = el("select", { class: "open-apps", style: { flex: "1" } }, el("option", { value: "" }, t("Pick an open app…")));
  const fill = async () => {
    const windows = await invoke("list_windows").catch(() => []);
    const taken = new Set(list.map((x) => x.toLowerCase()));
    const seen = new Set();
    const items = [];
    for (const win of windows) {
      const exe = win.exe_path.split(/[\\/]/).pop();
      const key = exe.toLowerCase();
      if (!exe || key === "shiori.exe" || seen.has(key) || taken.has(key)) {
        continue;
      }
      seen.add(key);
      items.push([exe, win.title]);
    }
    items.sort((a, b) => a[0].localeCompare(b[0]));
    picker.replaceChildren(
      el("option", { value: "" }, t(items.length ? "Pick an open app…" : "No other open app")),
      ...items.map(([exe, title]) => el("option", { value: exe }, title ? `${exe}  —  ${title}` : exe)),
    );
  };
  picker.onmousedown = fill;
  picker.onfocus = fill;
  picker.onchange = () => {
    if (picker.value) {
      addName(picker.value);
    }
  };
  fill();
  input.onkeydown = (event) => {
    if (event.key === "Enter") {
      add();
    }
  };

  return [
    hint(
      "Programs that are never detected as a visual novel. Their games are also hidden from the Library. " +
        "Browsers, Discord, Steam, OBS… are always ignored.",
    ),
    el("div", { class: "chips", style: { margin: "8px 0 14px" } }, chips),
    el("div", { class: "row" }, picker),
    el("div", { class: "row", style: { marginTop: "8px" } }, input, el("button", { class: "btn", onclick: add }, "Add")),
  ];
}

register({ id: "settings", fill: true, build, show: showPage });
