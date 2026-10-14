import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
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

type PlayableItemT =
  | { LocalFile: { path: string } }
  | { RadioStream: { url: string; name: string | null } }
  | { NavidromeTrack: { server_id: string; track_id: string } };

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

interface RadioStationT {
  id: string;
  name: string;
  url: string;
  country: string;
  language: string;
  codec: string;
  bitrate: number;
  favicon: string;
  clicks: number;
}

type PlayerEventT =
  | { Status: PlayerStatusT }
  | { QueueChanged: PlaylistEntryT[] }
  | { TrackStarted: PlaylistEntryT }
  | { TrackFinished: PlaylistEntryT }
  | { Warning: { message: string } }
  | { Fatal: { message: string } }
  | { StreamTitle: { title: string } };

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

interface FavoriteRecordT {
  kind: "local" | "navidrome" | "radio" | string;
  ref_id: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  duration_ms: number | null;
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

const localPath = (entry: PlaylistEntryT | null | undefined): string | null => {
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
let miniMode = false;
let theme: "retro" | "modern" = "retro";
let favorites: FavoriteRecordT[] = [];
let favFilter: "all" | "local" | "navidrome" | "radio" = "all";
let navidromeStreamResolver: ((ref: string) => Promise<string | null>) | null =
  null;
/** Live ICY title from the current radio stream (cleared on track change). */
let icyTitle: string | null = null;
let radioStations: RadioStationT[] = [];

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
    const radio = (current.item as { RadioStream?: { name: string | null } })
      .RadioStream;
    titleEl.textContent =
      radio && icyTitle ? `${radio.name ?? "Radio"} — ${icyTitle}` : displayTitle(current);
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
  renderFavChip();
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
    // Never hijack keys while typing in a form field (search, settings, …).
    const target = ev.target as HTMLElement | null;
    if (
      target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable)
    ) {
      return;
    }
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
  $("chip-mini").addEventListener("click", () => {
    miniMode = !miniMode;
    document.body.classList.toggle("mini", miniMode);
    $("chip-mini").classList.toggle("on", miniMode);
    const win = getCurrentWindow();
    const size = miniMode ? new LogicalSize(560, 250) : new LogicalSize(560, 520);
    void win.setSize(size).catch((e) => console.warn("resize failed", e));
  });

  $("chip-theme").addEventListener("click", () => {
    theme = theme === "retro" ? "modern" : "retro";
    document.documentElement.dataset.theme = theme;
    const chip = $("chip-theme");
    chip.textContent = `SKIN·${theme.toUpperCase()}`;
    chip.classList.toggle("on", theme === "modern");
  });

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

// ── Favorites ───────────────────────────────────────────────────────────────

function isCurrentFavorited(): boolean {
  const path = localPath(status?.current ?? null);
  return !!path && favorites.some((f) => f.kind === "local" && f.ref_id === path);
}

async function refreshFavorites(): Promise<void> {
  favorites = await invoke<FavoriteRecordT[]>("favorites_list", { kind: null });
  renderFavList();
  renderFavChip();
}

function renderFavChip(): void {
  const chip = $("chip-fav");
  const on = isCurrentFavorited();
  chip.textContent = on ? "♥" : "♡";
  chip.classList.toggle("on", on);
}

async function toggleCurrentFavorite(): Promise<void> {
  const current = status?.current;
  const path = localPath(current);
  if (!current || !path) {
    toast("Nothing to favorite");
    return;
  }
  const meta = metaByPath.get(path) ?? current.meta;
  const fav: FavoriteRecordT = {
    kind: "local",
    ref_id: path,
    title: meta.title ?? displayTitle(current),
    artist: meta.artist,
    album: meta.album,
    duration_ms: meta.duration ? Math.round(durSecs(meta.duration) * 1000) : null,
  };
  const nowFav = await invoke<boolean>("favorites_toggle", { fav });
  toast(nowFav ? "♥ Added to favorites" : "Removed from favorites");
  await refreshFavorites();
}

/** Build a playable entry from a favorite (navidrome resolves via the client). */
async function favToEntry(fav: FavoriteRecordT): Promise<PlaylistEntryT | null> {
  const meta: TrackMetaT = {
    ...emptyMeta(),
    title: fav.title,
    artist: fav.artist,
    album: fav.album,
    duration:
      fav.duration_ms != null
        ? makeDur(fav.duration_ms / 1000)
        : null,
  };
  if (fav.kind === "local") {
    return { item: { LocalFile: { path: fav.ref_id } }, meta };
  }
  if (fav.kind === "radio") {
    return {
      item: { RadioStream: { url: fav.ref_id, name: fav.title } },
      meta,
    };
  }
  if (fav.kind === "navidrome" && navidromeStreamResolver) {
    const url = await navidromeStreamResolver(fav.ref_id);
    if (!url) return null;
    return {
      item: { RadioStream: { url, name: fav.title } },
      meta,
    };
  }
  return null;
}

async function playFavorite(fav: FavoriteRecordT): Promise<void> {
  const entry = await favToEntry(fav);
  if (!entry) {
    toast("This favorite needs its source (Navidrome) configured", "error");
    return;
  }
  await send({ ReplaceQueue: { entries: [entry], start_at: 0 } });
  engineSynced = true;
  render();
}

function renderFavList(): void {
  const list = $("fav-list");
  list.replaceChildren();
  const shown = favorites.filter(
    (f) => favFilter === "all" || f.kind === favFilter,
  );
  if (shown.length === 0) {
    const li = document.createElement("li");
    li.textContent = favFilter === "all" ? "No favorites yet" : `No ${favFilter} favorites`;
    li.style.color = "var(--text-dim)";
    li.style.cursor = "default";
    list.appendChild(li);
    return;
  }
  for (const fav of shown) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.className = "fav-meta";
    const who = [fav.title ?? fav.ref_id.split("/").pop() ?? fav.ref_id, fav.artist]
      .filter(Boolean)
      .join(" · ");
    label.textContent = `${fav.kind === "local" ? "♪" : fav.kind === "radio" ? "📡" : "☁"} ${who}`;
    const un = document.createElement("button");
    un.className = "fav-un";
    un.textContent = "×";
    un.title = "Remove favorite";
    un.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      await invoke("favorites_toggle", { fav });
      await refreshFavorites();
    });
    li.append(label, un);
    li.title = fav.ref_id;
    li.addEventListener("click", () => void playFavorite(fav));
    list.appendChild(li);
  }
}

function bindFavorites(): void {
  $("chip-fav").addEventListener("click", () => void toggleCurrentFavorite());
  $("chip-favpanel").addEventListener("click", () => {
    const panel = $("fav-panel");
    panel.hidden = !panel.hidden;
    $("chip-favpanel").classList.toggle("on", !panel.hidden);
    if (!panel.hidden) void refreshFavorites();
  });
  document.querySelectorAll<HTMLButtonElement>("[data-fav-filter]").forEach((btn) => {
    btn.addEventListener("click", () => {
      favFilter = btn.dataset.favFilter as typeof favFilter;
      document
        .querySelectorAll("[data-fav-filter]")
        .forEach((b) => b.classList.toggle("on", b === btn));
      renderFavList();
    });
  });
  $("fav-export").addEventListener("click", () => {
    void (async () => {
      const dest = await save({
        filters: [{ name: "Favorites JSON", extensions: ["json"] }],
        defaultPath: "wynamp-favorites.json",
      });
      if (!dest) return;
      await invoke("favorites_export", { destPath: dest });
      toast("Favorites exported");
    })();
  });
  $("fav-import").addEventListener("click", () => {
    void (async () => {
      const src = await open({
        multiple: false,
        filters: [{ name: "Favorites JSON", extensions: ["json"] }],
      });
      if (!src) return;
      const added = await invoke<number>("favorites_import", {
        srcPath: Array.isArray(src) ? src[0] : src,
      });
      toast(`Imported ${added} new favorite${added === 1 ? "" : "s"}`);
      await refreshFavorites();
    })();
  });
}

// ── Internet radio ──────────────────────────────────────────────────────────

function radioEntry(station: {
  url: string;
  name: string | null;
}): PlaylistEntryT {
  return {
    item: { RadioStream: { url: station.url, name: station.name } },
    meta: emptyMeta(),
  };
}

async function playRadioEntry(entry: PlaylistEntryT): Promise<void> {
  icyTitle = null;
  await send({ ReplaceQueue: { entries: [entry], start_at: 0 } });
  engineSynced = true;
  render();
}

function renderRadioList(): void {
  const list = $("radio-list");
  list.replaceChildren();
  if (radioStations.length === 0) {
    const li = document.createElement("li");
    li.textContent = "Search for stations above";
    li.style.color = "var(--text-dim)";
    li.style.cursor = "default";
    list.appendChild(li);
    return;
  }
  for (const st of radioStations) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.className = "fav-meta";
    const bits = [st.name, st.country, st.codec, st.bitrate ? `${st.bitrate}k` : null]
      .filter(Boolean)
      .join(" · ");
    label.textContent = `📡 ${bits}`;
    const star = document.createElement("button");
    star.className = "fav-un";
    star.textContent = "♡";
    star.title = "Favorite this station";
    star.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const fav: FavoriteRecordT = {
        kind: "radio",
        ref_id: st.url,
        title: st.name,
        artist: null,
        album: null,
        duration_ms: null,
      };
      const nowFav = await invoke<boolean>("favorites_toggle", { fav });
      toast(nowFav ? "♥ Station favorited" : "Station unfavorited");
      await refreshFavorites();
    });
    li.append(label, star);
    li.title = st.url;
    li.addEventListener("click", () => {
      void playRadioEntry(radioEntry(st));
    });
    list.appendChild(li);
  }
}

function bindRadio(): void {
  $("chip-radio").addEventListener("click", () => {
    const panel = $("radio-panel");
    panel.hidden = !panel.hidden;
    $("chip-radio").classList.toggle("on", !panel.hidden);
  });

  $("radio-search").addEventListener("click", () => {
    void (async () => {
      try {
        toast("Searching stations…");
        radioStations = await invoke<RadioStationT[]>("radio_search", {
          name: ($("radio-name") as HTMLInputElement).value || null,
          country: ($("radio-country") as HTMLInputElement).value || null,
          language: null,
        });
        renderRadioList();
        toast(`${radioStations.length} stations`);
      } catch (error) {
        toast(`Search failed: ${String(error)}`, "error");
      }
    })();
  });

  $("radio-url").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") ($("radio-add") as HTMLElement).click();
  });
  $("radio-add").addEventListener("click", () => {
    void (async () => {
      const url = ($("radio-url") as HTMLInputElement).value.trim();
      if (!/^https?:\/\//.test(url)) {
        toast("Enter an http(s) stream URL", "error");
        return;
      }
      const name = url.split("/").slice(2).join("/") || "Custom stream";
      await playRadioEntry(radioEntry({ url, name }));
      // Remember it so it shows up in favorites if starred later.
    })();
  });

  renderRadioList();
}

// ── Navidrome ───────────────────────────────────────────────────────────────

interface NaviServerT {
  id: string;
  base_url: string;
  username: string;
}
interface NaviArtistT {
  id: string;
  name: string;
  album_count: number;
}
interface NaviAlbumT {
  id: string;
  name: string;
  artist: string;
  song_count: number;
  cover_id: string;
}
interface NaviSongT {
  id: string;
  title: string;
  artist: string;
  album: string;
  duration_secs: number;
  cover_id: string;
  bitrate: number | null;
}

let naviServers: NaviServerT[] = [];
let naviView: "artists" | "albums" | "songs" | "search" = "artists";
let naviArtistId: string | null = null;
let naviAlbumId: string | null = null;
let naviArtists: NaviArtistT[] = [];
let naviAlbums: NaviAlbumT[] = [];
let naviSongs: NaviSongT[] = [];
const coverCache = new Map<string, string>();
/** Scrobble target of the currently playing Navidrome song, if any. */
let lastNaviPlay: { serverId: string; songId: string; url: string } | null = null;

function naviServerId(): string | null {
  return ($("navi-server") as HTMLSelectElement).value || null;
}

async function refreshNaviServers(): Promise<void> {
  naviServers = await invoke<NaviServerT[]>("navidrome_servers");
  const select = $<HTMLSelectElement>("navi-server");
  select.replaceChildren();
  for (const srv of naviServers) {
    const opt = document.createElement("option");
    opt.value = srv.id;
    opt.textContent = `${srv.id} (${srv.base_url})`;
    select.appendChild(opt);
  }
  if (naviServers.length === 0) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = "No server — add in ⚙";
    select.appendChild(opt);
  }
  renderServerList();
}

function renderServerList(): void {
  const list = $("srv-list");
  list.replaceChildren();
  if (naviServers.length === 0) {
    const li = document.createElement("li");
    li.textContent = "No Navidrome servers configured";
    li.style.color = "var(--text-dim)";
    list.appendChild(li);
    return;
  }
  for (const srv of naviServers) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.className = "fav-meta";
    label.textContent = `☁ ${srv.id} · ${srv.username} @ ${srv.base_url}`;
    const test = document.createElement("button");
    test.className = "fav-un";
    test.textContent = "⇄";
    test.title = "Test connection";
    test.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      try {
        const version = await invoke<string>("navidrome_test", {
          serverId: srv.id,
        });
        toast(`✓ ${srv.id} OK (Subsonic ${version})`);
      } catch (error) {
        toast(`✗ ${String(error)}`, "error");
      }
    });
    const del = document.createElement("button");
    del.className = "fav-un";
    del.textContent = "×";
    del.title = "Remove server";
    del.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      await invoke("navidrome_remove_server", { id: srv.id });
      toast("Server removed");
      await refreshNaviServers();
    });
    li.append(label, test, del);
    list.appendChild(li);
  }
}

function bindSettings(): void {
  $("chip-settings").addEventListener("click", () => {
    const panel = $("settings-panel");
    panel.hidden = !panel.hidden;
    $("chip-settings").classList.toggle("on", !panel.hidden);
    if (!panel.hidden) void refreshNaviServers();
  });
  $("srv-add").addEventListener("click", () => {
    void (async () => {
      const id = ($("srv-id") as HTMLInputElement).value.trim();
      const base_url = ($("srv-url") as HTMLInputElement).value.trim();
      const username = ($("srv-user") as HTMLInputElement).value.trim();
      const password = ($("srv-pass") as HTMLInputElement).value;
      try {
        await invoke("navidrome_add_server", {
          id,
          baseUrl: base_url,
          username,
          password,
        });
        ($("srv-pass") as HTMLInputElement).value = "";
        toast("Server saved (password in keychain)");
        await refreshNaviServers();
      } catch (error) {
        toast(`Save failed: ${String(error)}`, "error");
      }
    })();
  });
}

async function loadCover(coverId: string): Promise<string> {
  if (!coverId) return "";
  const cached = coverCache.get(coverId);
  if (cached) return cached;
  const serverId = naviServerId();
  if (!serverId) return "";
  try {
    const b64 = await invoke<string>("navidrome_cover_b64", {
      serverId,
      coverId,
    });
    if (b64) coverCache.set(coverId, b64);
    return b64;
  } catch {
    return "";
  }
}

function naviRow(
  text: string,
  coverId: string | null,
  onClick: () => void,
): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "navi-row";
  if (coverId) {
    const img = document.createElement("img");
    img.alt = "";
    li.appendChild(img);
    void loadCover(coverId).then((b64) => {
      if (b64) img.src = `data:image/jpeg;base64,${b64}`;
    });
  }
  const label = document.createElement("span");
  label.className = "fav-meta";
  label.textContent = text;
  li.appendChild(label);
  li.addEventListener("click", onClick);
  return li;
}

async function playNaviSong(song: NaviSongT): Promise<void> {
  const serverId = naviServerId();
  if (!serverId) {
    toast("Select a server first", "error");
    return;
  }
  try {
    const url = await invoke<string>("navidrome_stream_url", {
      serverId,
      songId: song.id,
      maxBitrate: null,
    });
    const meta: TrackMetaT = {
      ...emptyMeta(),
      title: song.title,
      artist: song.artist,
      album: song.album,
      duration: song.duration_secs > 0 ? makeDur(song.duration_secs) : null,
      bitrate_kbps: song.bitrate,
    };
    icyTitle = null;
    await send({
      ReplaceQueue: {
        entries: [
          {
            item: { RadioStream: { url, name: song.title } },
            meta,
          },
        ],
        start_at: 0,
      },
    });
    engineSynced = true;
    lastNaviPlay = { serverId, songId: song.id, url };
    void invoke("navidrome_scrobble", {
      serverId,
      songId: song.id,
      submission: false,
    }).catch(() => undefined);
    render();
  } catch (error) {
    toast(`Play failed: ${String(error)}`, "error");
  }
}

function starNaviSong(song: NaviSongT): void {
  void (async () => {
    const serverId = naviServerId();
    if (!serverId) return;
    try {
      await invoke("navidrome_star", {
        serverId,
        songId: song.id,
        star: true,
      });
      const fav: FavoriteRecordT = {
        kind: "navidrome",
        ref_id: `${serverId}:${song.id}`,
        title: song.title,
        artist: song.artist,
        album: song.album,
        duration_ms: song.duration_secs * 1000 || null,
      };
      await invoke("favorites_toggle", { fav });
      toast("♥ Starred on server + favorites");
      await refreshFavorites();
    } catch (error) {
      toast(`Star failed: ${String(error)}`, "error");
    }
  })();
}

function renderNaviList(): void {
  const list = $("navi-list");
  list.replaceChildren();
  const back = $("navi-back") as HTMLButtonElement;
  back.disabled = naviView === "artists";

  if (naviView === "artists") {
    for (const a of naviArtists) {
      list.appendChild(
        naviRow(`${a.name} · ${a.album_count} albums`, null, () => {
          naviView = "albums";
          naviArtistId = a.id;
          void loadNaviAlbums();
        }),
      );
    }
    if (naviArtists.length === 0) {
      const li = document.createElement("li");
      li.textContent = naviServers.length ? "Loading / empty…" : "Add a server in ⚙ first";
      li.style.color = "var(--text-dim)";
      list.appendChild(li);
    }
  } else if (naviView === "albums") {
    for (const a of naviAlbums) {
      list.appendChild(
        naviRow(`${a.name} · ${a.artist} · ${a.song_count} tracks`, a.cover_id, () => {
          naviView = "songs";
          naviAlbumId = a.id;
          void loadNaviSongs();
        }),
      );
    }
  } else if (naviView === "songs") {
    for (const s of naviSongs) {
      const li = naviRow(`${s.title} · ${s.artist}`, s.cover_id, () => {
        void playNaviSong(s);
      });
      const star = document.createElement("button");
      star.className = "fav-un";
      star.textContent = "★";
      star.title = "Star (server + favorites)";
      star.addEventListener("click", (ev) => {
        ev.stopPropagation();
        starNaviSong(s);
      });
      li.appendChild(star);
      list.appendChild(li);
    }
  } else {
    // search: songs first, then albums, then artists
    const frag = document.createDocumentFragment();
    for (const s of naviSongs) {
      const li = naviRow(`♪ ${s.title} · ${s.artist} · ${s.album}`, s.cover_id, () => {
        void playNaviSong(s);
      });
      frag.appendChild(li);
    }
    for (const a of naviAlbums) {
      frag.appendChild(
        naviRow(`💿 ${a.name} · ${a.artist}`, a.cover_id, () => {
          naviView = "songs";
          naviAlbumId = a.id;
          void loadNaviSongs();
        }),
      );
    }
    for (const a of naviArtists) {
      frag.appendChild(
        naviRow(`👤 ${a.name}`, null, () => {
          naviView = "albums";
          naviArtistId = a.id;
          void loadNaviAlbums();
        }),
      );
    }
    list.appendChild(frag);
  }
}

async function loadNaviArtists(): Promise<void> {
  const serverId = naviServerId();
  if (!serverId) return;
  naviArtists = await invoke("navidrome_artists", { serverId });
  naviView = "artists";
  renderNaviList();
}

async function loadNaviAlbums(): Promise<void> {
  const serverId = naviServerId();
  if (!serverId) return;
  naviAlbums = await invoke("navidrome_albums", {
    serverId,
    artistId: naviArtistId,
  });
  renderNaviList();
}

async function loadNaviSongs(): Promise<void> {
  const serverId = naviServerId();
  if (!serverId || !naviAlbumId) return;
  naviSongs = await invoke("navidrome_songs", { serverId, albumId: naviAlbumId });
  renderNaviList();
}

function bindNavi(): void {
  $("chip-navi").addEventListener("click", () => {
    const panel = $("navi-panel");
    panel.hidden = !panel.hidden;
    $("chip-navi").classList.toggle("on", !panel.hidden);
    if (!panel.hidden) {
      void refreshNaviServers().then(() => {
        if (naviServers.length > 0) void loadNaviArtists();
        else renderNaviList();
      });
    }
  });
  $("navi-server").addEventListener("change", () => {
    void loadNaviArtists().catch((e) => toast(String(e), "error"));
  });
  $("navi-back").addEventListener("click", () => {
    if (naviView === "songs") {
      naviView = "albums";
      void loadNaviAlbums();
    } else if (naviView === "albums" || naviView === "search") {
      void loadNaviArtists();
    }
  });
  $("navi-search").addEventListener("click", () => {
    void (async () => {
      const serverId = naviServerId();
      const query = ($("navi-query") as HTMLInputElement).value.trim();
      if (!serverId || !query) return;
      try {
        const res = await invoke<{
          artists: NaviArtistT[];
          albums: NaviAlbumT[];
          songs: NaviSongT[];
        }>("navidrome_search", { serverId, query });
        naviArtists = res.artists;
        naviAlbums = res.albums;
        naviSongs = res.songs;
        naviView = "search";
        renderNaviList();
      } catch (error) {
        toast(`Search failed: ${String(error)}`, "error");
      }
    })();
  });
  $("navi-query").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") ($("navi-search") as HTMLElement).click();
  });
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
    } else if ("StreamTitle" in payload) {
      icyTitle = payload.StreamTitle.title;
      render();
    } else if ("TrackFinished" in payload) {
      // Scrobble a completed Navidrome play (matched by stream URL).
      if (lastNaviPlay) {
        const finished = payload.TrackFinished.item as {
          RadioStream?: { url: string };
        };
        if (finished.RadioStream?.url === lastNaviPlay.url) {
          void invoke("navidrome_scrobble", {
            serverId: lastNaviPlay.serverId,
            songId: lastNaviPlay.songId,
            submission: true,
          }).catch(() => undefined);
          lastNaviPlay = null;
        }
      }
    } else if ("TrackStarted" in payload) {
      icyTitle = null;
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
  bindFavorites();
  bindRadio();
  bindSettings();
  bindNavi();

  // Resolve Navidrome favorites ("server:song") into playable stream URLs.
  navidromeStreamResolver = async (ref) => {
    const sep = ref.indexOf(":");
    if (sep < 0) return null;
    const serverId = ref.slice(0, sep);
    const songId = ref.slice(sep + 1);
    try {
      return await invoke<string>("navidrome_stream_url", {
        serverId,
        songId,
        maxBitrate: null,
      });
    } catch (error) {
      toast(`Cannot resolve favorite: ${String(error)}`, "error");
      return null;
    }
  };

  await refreshFavorites().catch((e) => console.warn("favorites load failed", e));

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
