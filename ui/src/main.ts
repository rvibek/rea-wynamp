import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

// ── Wire types (mirror of the Rust serde shapes) ────────────────────────────

interface Duration {
  secs: number;
  nanos: number;
}

interface TrackMetaT {
  title: string | null;
  artist: string | null;
  album: string | null;
  album_artist: string | null;
  genre: string | null;
  track_no: number | null;
  disc_no: number | null;
  year: number | null;
  duration: Duration | null;
  bitrate_kbps: number | null;
  sample_rate_hz: number | null;
  channels: number | null;
  codec: string | null;
}

type PlayableItemT = { LocalFile: { path: string } } | Record<string, never>;

interface PlaylistEntryT {
  item: PlayableItemT;
  meta: TrackMetaT;
}

interface PlayerStatusT {
  state: "Stopped" | "Playing" | "Paused" | "Buffering";
  current: PlaylistEntryT | null;
  position: Duration;
  duration: Duration | null;
  volume: number;
  balance: number;
  shuffle: boolean;
  repeat: "Off" | "All" | "One";
  bitrate_kbps: number | null;
  sample_rate_hz: number | null;
  eq_enabled: boolean;
  eq_gains_db: number[];
  preamp_db: number;
}

type PlayerEventT =
  | { Status: PlayerStatusT }
  | { QueueChanged: PlaylistEntryT[] }
  | { TrackStarted: PlaylistEntryT }
  | { TrackFinished: PlaylistEntryT }
  | { Warning: { message: string } }
  | { Fatal: { message: string } };

interface TrackInfoT {
  path: string;
  codec: string;
  meta: TrackMetaT;
}

interface ScanProgressT {
  found: number;
  path: string;
}

interface ScanDoneT {
  total: number;
  paths: string[];
}

interface ImportedPlaylistT {
  name: string;
  entries: PlaylistEntryT[];
}

// ── Helpers ─────────────────────────────────────────────────────────────────

const durSecs = (d: Duration | null | undefined): number =>
  d ? d.secs + d.nanos / 1e9 : 0;

const makeDur = (seconds: number): Duration => ({
  secs: Math.max(0, Math.floor(seconds)),
  nanos: Math.max(0, Math.round((seconds % 1) * 1e9)),
});

const fmtTime = (seconds: number): string => {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
};

const emptyMeta = (): TrackMetaT => ({
  title: null,
  artist: null,
  album: null,
  album_artist: null,
  genre: null,
  track_no: null,
  disc_no: null,
  year: null,
  duration: null,
  bitrate_kbps: null,
  sample_rate_hz: null,
  channels: null,
  codec: null,
});

const localPath = (entry: PlaylistEntryT | null): string | null => {
  if (!entry) return null;
  const item = entry.item as { LocalFile?: { path: string } };
  return item.LocalFile?.path ?? null;
};

/** Mirrors PlaylistEntry::display_title in the core crate. */
function displayTitle(entry: PlaylistEntryT): string {
  const title = entry.meta.title?.trim();
  if (title) return title;
  const path = localPath(entry);
  if (path) {
    const name = path.split("/").pop() ?? path;
    return name.replace(/\.[^.]+$/, "") || "Unknown";
  }
  return "Unknown";
}

const entryFor = (path: string): PlaylistEntryT => ({
  item: { LocalFile: { path } },
  meta: metaByPath.get(path) ?? emptyMeta(),
});

// ── State ───────────────────────────────────────────────────────────────────

let status: PlayerStatusT | null = null;
let queue: PlaylistEntryT[] = [];
const metaByPath = new Map<string, TrackMetaT>();
const missingPaths = new Set<string>();
const selected = new Set<number>();
let dragFrom: number | null = null;
let engineSynced = false;
let draggingSeek = false;
let dragSeekFrac = 0;
let draggingVolume = false;
let draggingBalance = false;
let scanning = false;

// Equalizer + visualizer
let eqEnabled = true;
let eqGains: number[] = Array(10).fill(0);
let preampDb = 0;
let draggingEq = false;
let vizMode: "off" | "spec" | "scope" = "off";

const EQ_LABELS = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
const PRESETS: Record<string, number[]> = {
  Flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  Rock: [5, 4, 3, 1, -1, -1, 0, 2, 3, 4],
  Pop: [-1, 1, 3, 4, 3, 0, -1, -1, 1, 2],
  Jazz: [3, 2, 1, 2, -1, -1, 0, 1, 3, 4],
  Classical: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4],
  "Bass boost": [7, 6, 5, 3, 1, 0, 0, 0, 0, 0],
  "Treble boost": [0, 0, 0, 0, 0, 1, 3, 5, 6, 7],
  Club: [0, 0, 4, 3, 2, 2, 1, 0, 0, 0],
};

const send = (command: unknown): Promise<void> =>
  invoke("player_command", { command });

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
};

/** Send a playback-affecting command, syncing the engine queue first if needed. */
async function playback(command: unknown): Promise<void> {
  if (queue.length > 0 && !engineSynced) {
    await send({ ReplaceQueue: { entries: queue, start_at: 0 } });
    engineSynced = true;
  }
  await send(command);
}

function persistSession(): void {
  const paths = queue
    .map(localPath)
    .filter((p): p is string => typeof p === "string");
  void invoke("library_save_session", { paths }).catch((e) =>
    console.warn("session save failed", e),
  );
}

async function metaForPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  const infos = await invoke<TrackInfoT[]>("read_track_meta", { paths });
  infos.forEach((info) => metaByPath.set(info.path, info.meta));
}

// ── Render ──────────────────────────────────────────────────────────────────

function render(): void {
  const st = status;
  const badge = $("state-badge");
  badge.textContent = st ? st.state.toLowerCase() : "stopped";
  badge.dataset.state = st?.state ?? "Stopped";

  const current = st?.current ?? null;
  const titleEl = $("track-title");
  const detailEl = $("track-detail");
  const techEl = $("track-tech");

  if (current) {
    titleEl.textContent = displayTitle(current);
    const parts = [current.meta.artist, current.meta.album].filter(Boolean);
    detailEl.textContent = parts.length ? parts.join(" — ") : "—";
    const meta = metaByPath.get(localPath(current) ?? "") ?? current.meta;
    const kbps = st?.bitrate_kbps ?? meta.bitrate_kbps;
    const khz = st?.sample_rate_hz ?? meta.sample_rate_hz;
    const codec = meta.codec ?? "";
    techEl.textContent =
      [
        kbps ? `${kbps} kbps` : null,
        khz ? `${(khz / 1000).toFixed(khz % 1000 === 0 ? 0 : 1)} kHz` : null,
        codec ? codec.toUpperCase() : null,
      ]
        .filter(Boolean)
        .join(" · ") || "—";
  } else {
    titleEl.textContent = queue.length
      ? "Ready — press Play"
      : "No track — open files or a folder";
    detailEl.textContent = "—";
    techEl.textContent = "-- kbps · -- kHz";
  }

  const elapsed = draggingSeek
    ? dragSeekFrac * totalSeconds()
    : durSecs(st?.position);
  const total = totalSeconds();
  $("time-elapsed").textContent = fmtTime(elapsed);
  $("time-total").textContent = fmtTime(total);
  const frac = total > 0 ? Math.min(1, elapsed / total) : 0;
  $("seek-fill").style.width = `${frac * 100}%`;
  $("seek-knob").style.left = `${frac * 100}%`;

  const shuffle = $("chip-shuffle");
  shuffle.classList.toggle("on", !!st?.shuffle);
  const repeat = $("chip-repeat");
  const rep = st?.repeat ?? "Off";
  repeat.textContent = `REP·${rep.toUpperCase()}`;
  repeat.classList.toggle("on", rep !== "Off");

  if (!draggingVolume) {
    ($("volume") as HTMLInputElement).value = String(
      Math.round((st?.volume ?? 1) * 100),
    );
  }
  if (!draggingBalance) {
    ($("balance") as HTMLInputElement).value = String(
      Math.round((st?.balance ?? 0) * 100),
    );
  }

  renderQueue();
  updateMarquee();
  syncEqUi(st);
}

/** Mirror engine EQ state into the panel (unless the user is mid-drag). */
function syncEqUi(st: PlayerStatusT | null): void {
  if (!st || draggingEq) return;
  const changed =
    st.eq_enabled !== eqEnabled ||
    st.preamp_db !== preampDb ||
    st.eq_gains_db.some((g, i) => g !== eqGains[i]);
  if (!changed) return;
  eqEnabled = st.eq_enabled;
  preampDb = st.preamp_db;
  eqGains = [...st.eq_gains_db];
  const onChip = $("chip-eq-on");
  onChip.textContent = eqEnabled ? "EQ·ON" : "EQ·OFF";
  onChip.classList.toggle("on", eqEnabled);
  ($("eq-preamp") as HTMLInputElement).value = String(preampDb);
  document.querySelectorAll<HTMLInputElement>("#eq-bands input").forEach((el, i) => {
    el.value = String(eqGains[i] ?? 0);
  });
}

function totalSeconds(): number {
  const st = status;
  if (!st) return 0;
  if (st.duration && durSecs(st.duration) > 0) return durSecs(st.duration);
  const path = localPath(st.current);
  const meta = path ? metaByPath.get(path) : null;
  if (meta?.duration) return durSecs(meta.duration);
  return 0;
}

function entryDuration(entry: PlaylistEntryT): number {
  if (entry.meta.duration) return durSecs(entry.meta.duration);
  const path = localPath(entry);
  const meta = path ? metaByPath.get(path) : null;
  return meta?.duration ? durSecs(meta.duration) : 0;
}

function renderQueue(): void {
  const list = $("queue-list");
  const currentPath = localPath(status?.current ?? null);
  list.replaceChildren();

  queue.forEach((entry, index) => {
    const li = document.createElement("li");
    const path = localPath(entry);
    li.textContent = `${index + 1}. ${displayTitle(entry)}`;
    if (path && path === currentPath) li.classList.add("current");
    if (selected.has(index)) li.classList.add("selected");
    if (path && missingPaths.has(path)) li.classList.add("missing");
    li.title = path ?? "";
    li.draggable = true;

    li.addEventListener("click", (ev) => {
      if (ev.metaKey || ev.ctrlKey) {
        if (selected.has(index)) selected.delete(index);
        else selected.add(index);
        renderQueue();
        return;
      }
      selected.clear();
      jumpTo(index);
    });

    li.addEventListener("dragstart", (ev) => {
      dragFrom = index;
      ev.dataTransfer?.setData("text/plain", String(index));
      ev.dataTransfer!.effectAllowed = "move";
    });
    li.addEventListener("dragover", (ev) => {
      ev.preventDefault();
      li.classList.add("dragover");
    });
    li.addEventListener("dragleave", () => li.classList.remove("dragover"));
    li.addEventListener("drop", (ev) => {
      ev.preventDefault();
      li.classList.remove("dragover");
      if (dragFrom === null || dragFrom === index) return;
      void moveEntry(dragFrom, index);
      dragFrom = null;
    });
    li.addEventListener("dragend", () => {
      dragFrom = null;
      list
        .querySelectorAll(".dragover")
        .forEach((el) => el.classList.remove("dragover"));
    });

    list.appendChild(li);
  });

  // Summary line: count + total known duration.
  const count = queue.length;
  const seconds = queue.reduce((sum, e) => sum + entryDuration(e), 0);
  $("queue-summary").textContent = `${count} ${count === 1 ? "track" : "tracks"} · ${fmtTime(seconds)}`;
}

async function jumpTo(index: number): Promise<void> {
  if (index < 0 || index >= queue.length) return;
  await send({ ReplaceQueue: { entries: queue, start_at: index } });
  engineSynced = true;
  render();
}

async function moveEntry(from: number, to: number): Promise<void> {
  if (from === to || from < 0 || to < 0 || from >= queue.length || to >= queue.length) {
    return;
  }
  const [moved] = queue.splice(from, 1);
  queue.splice(to, 0, moved);
  selected.clear();
  if (engineSynced) {
    await send({ MoveEntry: { from, to } });
  }
  persistSession();
  render();
}

function updateMarquee(): void {
  const box = $("marquee");
  box.classList.toggle("scrolling", box.scrollWidth > box.clientWidth + 4);
}

// ── Toast ───────────────────────────────────────────────────────────────────

let toastTimer: number | undefined;
function toast(message: string, kind: "info" | "error" = "info"): void {
  const el = $("toast");
  el.textContent = message;
  el.dataset.kind = kind;
  el.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    el.hidden = true;
  }, kind === "error" ? 6000 : 3500);
}

// ── Opening files / folders / playlists ─────────────────────────────────────

async function loadAndPlay(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  try {
    await metaForPaths(paths);
    const entries = paths.map(entryFor);
    await send({ ReplaceQueue: { entries, start_at: 0 } });
    engineSynced = true;
    selected.clear();
    persistSession();
    render();
  } catch (error) {
    toast(`Could not open files: ${String(error)}`, "error");
  }
}

async function openFiles(): Promise<void> {
  try {
    const picked = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "Audio", extensions: [
        "mp3", "m4a", "m4b", "aac", "flac", "wav", "aiff", "aif", "ogg", "oga",
        "opus", "alac", "mka", "mp4",
      ] }],
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    await loadAndPlay(paths);
  } catch (error) {
    toast(`Open failed: ${String(error)}`, "error");
  }
}

async function openFolder(): Promise<void> {
  if (scanning) {
    toast("A scan is already running");
    return;
  }
  try {
    const picked = await open({ directory: true, multiple: false });
    if (!picked) return;
    const path = Array.isArray(picked) ? picked[0] : picked;
    scanning = true;
    toast("Scanning folder…");
    await invoke("library_scan_start", { path });
  } catch (error) {
    scanning = false;
    toast(`Scan failed: ${String(error)}`, "error");
  }
}

async function savePlaylist(): Promise<void> {
  if (queue.length === 0) {
    toast("Queue is empty");
    return;
  }
  try {
    const dest = await save({
      filters: [{ name: "Playlist", extensions: ["m3u8", "m3u", "pls"] }],
      defaultPath: "playlist.m3u8",
    });
    if (!dest) return;
    await invoke("playlist_export", { destPath: dest, entries: queue });
    toast(`Saved ${queue.length} tracks`);
  } catch (error) {
    toast(`Save failed: ${String(error)}`, "error");
  }
}

async function loadPlaylist(): Promise<void> {
  try {
    const src = await open({
      multiple: false,
      filters: [{ name: "Playlist", extensions: ["m3u8", "m3u", "pls"] }],
    });
    if (!src) return;
    const srcPath = Array.isArray(src) ? src[0] : src;
    const imported = await invoke<ImportedPlaylistT>("playlist_import", {
      srcPath,
    });
    if (imported.entries.length === 0) {
      toast("Playlist has no entries", "error");
      return;
    }
    // Enrich entries with on-disk tags where the playlist had none.
    const paths = imported.entries
      .map(localPath)
      .filter((p): p is string => typeof p === "string");
    await metaForPaths(paths);
    const entries = imported.entries.map((entry) => {
      const path = localPath(entry);
      const disk = path ? metaByPath.get(path) : null;
      if (!disk) return entry;
      return {
        item: entry.item,
        meta: { ...disk, ...entry.meta, duration: entry.meta.duration ?? disk.duration },
      };
    });
    queue = entries;
    await send({ ReplaceQueue: { entries, start_at: 0 } });
    engineSynced = true;
    selected.clear();
    persistSession();
    render();
    toast(`Loaded “${imported.name}” (${entries.length} tracks)`);
  } catch (error) {
    toast(`Load failed: ${String(error)}`, "error");
  }
}

// ── Seek bar interaction ────────────────────────────────────────────────────

function seekFracFromEvent(ev: PointerEvent): number {
  const rect = $("seek").getBoundingClientRect();
  const x = ev.clientX - rect.left;
  return Math.min(1, Math.max(0, x / rect.width));
}

function bindSeek(): void {
  const seek = $("seek");
  seek.addEventListener("pointerdown", (ev) => {
    if (totalSeconds() <= 0) return;
    draggingSeek = true;
    dragSeekFrac = seekFracFromEvent(ev);
    seek.setPointerCapture(ev.pointerId);
    render();
  });
  seek.addEventListener("pointermove", (ev) => {
    if (!draggingSeek) return;
    dragSeekFrac = seekFracFromEvent(ev);
    render();
  });
  const finish = (ev: PointerEvent): void => {
    if (!draggingSeek) return;
    draggingSeek = false;
    dragSeekFrac = seekFracFromEvent(ev);
    const target = dragSeekFrac * totalSeconds();
    void send({ Seek: makeDur(target) });
    render();
  };
  seek.addEventListener("pointerup", finish);
  seek.addEventListener("pointercancel", () => {
    draggingSeek = false;
    render();
  });
}

// ── Sliders ─────────────────────────────────────────────────────────────────

function bindSliders(): void {
  const volume = $<HTMLInputElement>("volume");
  volume.addEventListener("input", () => {
    draggingVolume = true;
    void send({ SetVolume: Number(volume.value) / 100 });
  });
  volume.addEventListener("change", () => {
    draggingVolume = false;
    render();
  });

  const balance = $<HTMLInputElement>("balance");
  balance.addEventListener("input", () => {
    draggingBalance = true;
    void send({ SetBalance: Number(balance.value) / 100 });
  });
  balance.addEventListener("change", () => {
    draggingBalance = false;
    render();
  });
}

// ── Buttons & keyboard ──────────────────────────────────────────────────────

/** Empty the queue. The library (and session) keep their files. */
async function clearQueue(): Promise<void> {
  if (queue.length === 0) return;
  queue = [];
  selected.clear();
  missingPaths.clear();
  await send("ClearQueue");
  engineSynced = true; // engine queue is now empty too
  persistSession();
  render();
  toast("Queue cleared (library kept)");
}

async function deleteSelected(): Promise<void> {
  if (selected.size === 0) return;
  const indices = [...selected].sort((a, b) => b - a);
  for (const index of indices) {
    if (index >= queue.length) continue;
    queue.splice(index, 1);
    if (engineSynced) await send({ RemoveAt: index });
  }
  selected.clear();
  persistSession();
  render();
}

function bindControls(): void {
  $("btn-prev").addEventListener("click", () => void playback("Previous"));
  $("btn-play").addEventListener("click", () => void playback("Play"));
  $("btn-pause").addEventListener("click", () => void send("Pause"));
  $("btn-stop").addEventListener("click", () => void send("Stop"));
  $("btn-next").addEventListener("click", () => void playback("Next"));
  $("btn-open").addEventListener("click", () => void openFiles());
  $("btn-folder").addEventListener("click", () => void openFolder());
  $("btn-save").addEventListener("click", () => void savePlaylist());
  $("btn-load").addEventListener("click", () => void loadPlaylist());
  $("btn-clear").addEventListener("click", () => void clearQueue());

  $("chip-shuffle").addEventListener("click", () => {
    void send({ SetShuffle: !(status?.shuffle ?? false) });
  });

  const cycle = { Off: "All", All: "One", One: "Off" } as const;
  $("chip-repeat").addEventListener("click", () => {
    const next = cycle[status?.repeat ?? "Off"];
    void send({ SetRepeat: next });
  });

  window.addEventListener("keydown", (ev) => {
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
    switch (ev.key) {
      case " ":
        ev.preventDefault();
        void playback("Toggle");
        break;
      case "Delete":
      case "Backspace":
        if (selected.size > 0) {
          ev.preventDefault();
          void deleteSelected();
        }
        break;
      case "z":
      case "Z":
        void playback("Previous");
        break;
      case "x":
      case "X":
        void playback("Play");
        break;
      case "c":
      case "C":
        void send("Pause");
        break;
      case "v":
      case "V":
        void send("Stop");
        break;
      case "b":
      case "B":
        void playback("Next");
        break;
      case "ArrowLeft":
        ev.preventDefault();
        void send({ SeekRelative: -5000 });
        break;
      case "ArrowRight":
        ev.preventDefault();
        void send({ SeekRelative: 5000 });
        break;
      case "ArrowUp":
        ev.preventDefault();
        void send({ VolumeRelative: 0.05 });
        break;
      case "ArrowDown":
        ev.preventDefault();
        void send({ VolumeRelative: -0.05 });
        break;
      default:
        break;
    }
  });
}

// ── Drag & drop of files ────────────────────────────────────────────────────

const DROP_EXTENSIONS = [
  "mp3", "m4a", "m4b", "aac", "flac", "wav", "aiff", "aif", "ogg", "oga",
  "opus", "alac", "mka", "mp4",
];

function bindDrop(): void {
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== "drop") return;
    const paths = event.payload.paths.filter((path) => {
      const ext = path.split(".").pop()?.toLowerCase() ?? "";
      return DROP_EXTENSIONS.includes(ext);
    });
    if (paths.length === 0) return;
    void (async () => {
      if (queue.length === 0 || !engineSynced) {
        // First batch: replace (plays immediately, like OPEN).
        await loadAndPlay(paths);
      } else {
        // Later batches: append to the live queue.
        await metaForPaths(paths);
        const entries = paths.map(entryFor);
        queue.push(...entries);
        await send({ Enqueue: entries });
        persistSession();
        render();
      }
    })();
  });
}

// ── Scan events ─────────────────────────────────────────────────────────────

function bindScanEvents(): void {
  void listen<ScanProgressT>("scan:event", ({ payload }) => {
    const name = payload.path.split("/").pop() ?? payload.path;
    toast(`Scanning… ${payload.found} found (latest: ${name})`);
  });

  void listen<ScanDoneT>("scan:done", ({ payload }) => {
    scanning = false;
    void (async () => {
      const { total, paths } = payload;
      if (total === 0) {
        toast("No audio files found in that folder");
        return;
      }
      await metaForPaths(paths);
      paths.forEach((p) => missingPaths.delete(p));
      const entries = paths.map(entryFor);

      if (queue.length === 0) {
        queue = entries;
        await send({ ReplaceQueue: { entries, start_at: 0 } });
        engineSynced = true;
      } else {
        queue.push(...entries);
        if (engineSynced) await send({ Enqueue: entries });
        // If the engine wasn't synced yet, the restored queue + scan will be
        // pushed together on first Play.
      }
      persistSession();
      render();
      toast(`Added ${total} tracks`);
    })();
  });

  void listen<string>("scan:error", ({ payload }) => {
    scanning = false;
    toast(`Scan failed: ${payload}`, "error");
  });
}

// ── Equalizer & visualizer UI ───────────────────────────────────────────────

let eqDebounce: number | undefined;

function scheduleEqGains(): void {
  window.clearTimeout(eqDebounce);
  eqDebounce = window.setTimeout(() => {
    void send({ SetEqGains: eqGains });
  }, 50);
}

function buildEqPanel(): void {
  const bandsEl = $("eq-bands");
  bandsEl.replaceChildren();
  EQ_LABELS.forEach((label, i) => {
    const wrap = document.createElement("div");
    wrap.className = "eq-band";
    const input = document.createElement("input");
    input.type = "range";
    input.min = "-14";
    input.max = "14";
    input.step = "0.5";
    input.value = String(eqGains[i] ?? 0);
    input.addEventListener("input", () => {
      draggingEq = true;
      eqGains[i] = Number(input.value);
      scheduleEqGains();
    });
    input.addEventListener("change", () => {
      draggingEq = false;
    });
    const cap = document.createElement("span");
    cap.textContent = label;
    wrap.append(input, cap);
    bandsEl.appendChild(wrap);
  });

  const preset = $<HTMLSelectElement>("eq-preset");
  preset.replaceChildren();
  for (const name of Object.keys(PRESETS)) {
    const opt = document.createElement("option");
    opt.value = name;
    opt.textContent = name;
    preset.appendChild(opt);
  }
  preset.addEventListener("change", () => {
    const gains = PRESETS[preset.value];
    if (!gains) return;
    eqGains = [...gains];
    bandsEl.querySelectorAll<HTMLInputElement>("input").forEach((el, i) => {
      el.value = String(eqGains[i] ?? 0);
    });
    void send({ SetEqGains: eqGains });
  });
}

function bindEqControls(): void {
  $("chip-eq").addEventListener("click", () => {
    const panel = $("eq-panel");
    panel.hidden = !panel.hidden;
    $("chip-eq").classList.toggle("on", !panel.hidden);
  });
  $("chip-eq-on").addEventListener("click", () => {
    eqEnabled = !eqEnabled;
    void send({ SetEqEnabled: eqEnabled });
    $("chip-eq-on").textContent = eqEnabled ? "EQ·ON" : "EQ·OFF";
    $("chip-eq-on").classList.toggle("on", eqEnabled);
  });
  $("eq-preamp").addEventListener("input", (ev) => {
    preampDb = Number((ev.target as HTMLInputElement).value);
    void send({ SetPreamp: preampDb });
  });
}

function bindViz(): void {
  $("chip-viz").addEventListener("click", () => {
    vizMode = vizMode === "off" ? "spec" : vizMode === "spec" ? "scope" : "off";
    const chip = $("chip-viz");
    chip.textContent = `VIZ·${vizMode.toUpperCase()}`;
    chip.classList.toggle("on", vizMode !== "off");
    $("viz").hidden = vizMode === "off";
  });

  const canvas = $<HTMLCanvasElement>("viz");
  const ctx = canvas.getContext("2d");
  window.setInterval(() => {
    if (vizMode === "off" || !ctx) return;
    void (async () => {
      try {
        const snap = await invoke<{ wave: number[]; spectrum: number[] }>(
          "visualizer_snapshot",
          { bands: 28, wavePoints: canvas.width },
        );
        ctx.fillStyle = "#050506";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        if (vizMode === "spec") {
          const n = snap.spectrum.length;
          if (n === 0) return;
          const gap = 2;
          const barW = (canvas.width - gap * (n - 1)) / n;
          ctx.fillStyle = "#39ff14";
          snap.spectrum.forEach((v, i) => {
            const h = Math.max(1, v * (canvas.height - 2));
            ctx.fillRect(i * (barW + gap), canvas.height - h, barW, h);
          });
        } else {
          ctx.strokeStyle = "#39ff14";
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          const mid = canvas.height / 2;
          snap.wave.forEach((v, i) => {
            const x = (i / Math.max(1, snap.wave.length - 1)) * canvas.width;
            const y = mid - v * (canvas.height / 2 - 2);
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          });
          ctx.stroke();
        }
      } catch (error) {
        console.warn("visualizer snapshot failed", error);
      }
    })();
  }, 33);
}

// ── Boot ────────────────────────────────────────────────────────────────────

async function boot(): Promise<void> {
  $("version").textContent = `v${await invoke<string>("app_version")}`;

  await listen<PlayerEventT>("player:event", ({ payload }) => {
    if ("Status" in payload) {
      status = payload.Status;
      render();
    } else if ("QueueChanged" in payload) {
      // Engine-side queue is authoritative while playing; we already mirror
      // locally on every mutation, so just re-render the highlight.
      render();
    } else if ("Warning" in payload) {
      toast(payload.Warning.message);
    } else if ("Fatal" in payload) {
      toast(payload.Fatal.message, "error");
    } else if ("TrackStarted" in payload) {
      toast(`▶ ${displayTitle(payload.TrackStarted)}`);
    }
  });

  bindScanEvents();
  bindSeek();
  bindSliders();
  bindControls();
  bindDrop();
  buildEqPanel();
  bindEqControls();
  bindViz();

  // Restore the last session queue (display only — no autoplay, engine idle).
  try {
    const sessionPaths = await invoke<string[]>("library_session_load");
    if (sessionPaths.length > 0) {
      await metaForPaths(sessionPaths);
      queue = sessionPaths.map(entryFor);
    }
  } catch (error) {
    console.warn("session restore failed", error);
  }

  // Library status: total tracks + anything that went missing.
  try {
    const lib = await invoke<{ track_count: number; missing: string[] }>(
      "library_status",
    );
    lib.missing.forEach((p) => missingPaths.add(p));
    if (lib.missing.length > 0) {
      toast(
        `${lib.missing.length} library file${lib.missing.length === 1 ? " is" : "s are"} missing`,
        "error",
      );
    }
    if (lib.track_count > 0 && sessionPathsSafe() === 0) {
      // Quiet hint that the library exists even with an empty queue.
      console.info(`library: ${lib.track_count} tracks`);
    }
  } catch (error) {
    console.warn("library status failed", error);
  }

  await send("QueryStatus");
  render();
  window.addEventListener("resize", updateMarquee);
}

function sessionPathsSafe(): number {
  return queue.length;
}

void boot();
