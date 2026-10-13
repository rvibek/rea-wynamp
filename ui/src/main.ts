import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
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

const AUDIO_EXTENSIONS = [
  "mp3", "m4a", "aac", "flac", "wav", "aiff", "aif", "ogg", "oga", "opus",
  "alac", "mka", "mp4",
];

// ── State ───────────────────────────────────────────────────────────────────

let status: PlayerStatusT | null = null;
let queue: PlaylistEntryT[] = [];
const metaByPath = new Map<string, TrackMetaT>();
let draggingSeek = false;
let dragSeekFrac = 0;

const send = (command: unknown): Promise<void> =>
  invoke("player_command", { command });

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
};

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
    techEl.textContent = [
      kbps ? `${kbps} kbps` : null,
      khz ? `${(khz / 1000).toFixed(khz % 1000 === 0 ? 0 : 1)} kHz` : null,
      codec ? codec.toUpperCase() : null,
    ]
      .filter(Boolean)
      .join(" · ") || "—";
  } else {
    titleEl.textContent = "No track — open some files";
    detailEl.textContent = "—";
    techEl.textContent = "-- kbps · -- kHz";
  }

  // Times + seek bar
  const elapsed = draggingSeek
    ? dragSeekFrac * totalSeconds()
    : durSecs(st?.position);
  const total = totalSeconds();
  $("time-elapsed").textContent = fmtTime(elapsed);
  $("time-total").textContent = fmtTime(total);
  const frac = total > 0 ? Math.min(1, elapsed / total) : 0;
  $("seek-fill").style.width = `${frac * 100}%`;
  $("seek-knob").style.left = `${frac * 100}%`;

  // Toggles & sliders
  const shuffle = $("chip-shuffle");
  shuffle.classList.toggle("on", !!st?.shuffle);
  const repeat = $("chip-repeat");
  const rep = st?.repeat ?? "Off";
  repeat.textContent = `REP·${rep.toUpperCase()}`;
  repeat.classList.toggle("on", rep !== "Off");

  if (!draggingVolume) {
    const volume = $("volume") as HTMLInputElement;
    volume.value = String(Math.round((st?.volume ?? 1) * 100));
  }
  if (!draggingBalance) {
    const balance = $("balance") as HTMLInputElement;
    balance.value = String(Math.round((st?.balance ?? 0) * 100));
  }

  renderQueue();
  updateMarquee();
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

function renderQueue(): void {
  const list = $("queue-list");
  const currentPath = localPath(status?.current ?? null);
  list.replaceChildren();
  queue.forEach((entry, index) => {
    const li = document.createElement("li");
    li.textContent = `${index + 1}. ${displayTitle(entry)}`;
    if (localPath(entry) === currentPath) li.classList.add("current");
    li.title = localPath(entry) ?? "";
    li.addEventListener("click", () => {
      // Jump to this track (engine hard-cuts; queue stays intact).
      void send({ ReplaceQueue: { entries: queue, start_at: index } });
    });
    list.appendChild(li);
  });
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

// ── Opening files ───────────────────────────────────────────────────────────

async function loadAndPlay(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  try {
    const infos = await invoke<TrackInfoT[]>("read_track_meta", { paths });
    infos.forEach((info) => metaByPath.set(info.path, info.meta));
    const entries: PlaylistEntryT[] = paths.map((path) => ({
      item: { LocalFile: { path } },
      meta: metaByPath.get(path) ?? emptyMeta(),
    }));
    await send({ ReplaceQueue: { entries, start_at: 0 } });
  } catch (error) {
    toast(`Could not open files: ${String(error)}`, "error");
  }
}

async function openFiles(): Promise<void> {
  try {
    const picked = await open({
      multiple: true,
      directory: false,
      filters: [
        {
          name: "Audio",
          extensions: AUDIO_EXTENSIONS,
        },
      ],
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    await loadAndPlay(paths);
  } catch (error) {
    toast(`Open failed: ${String(error)}`, "error");
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

let draggingVolume = false;
let draggingBalance = false;

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

function bindControls(): void {
  $("btn-prev").addEventListener("click", () => void send("Previous"));
  $("btn-play").addEventListener("click", () => void send("Play"));
  $("btn-pause").addEventListener("click", () => void send("Pause"));
  $("btn-stop").addEventListener("click", () => void send("Stop"));
  $("btn-next").addEventListener("click", () => void send("Next"));
  $("btn-open").addEventListener("click", () => void openFiles());

  $("chip-shuffle").addEventListener("click", () => {
    void send({ SetShuffle: !(status?.shuffle ?? false) });
  });

  const cycle = { Off: "All", All: "One", One: "Off" } as const;
  $("chip-repeat").addEventListener("click", () => {
    const next = cycle[status?.repeat ?? "Off"];
    void send({ SetRepeat: next });
  });

  window.addEventListener("keydown", (ev) => {
    // Don't hijack when modifiers are held (devtools etc.).
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
    switch (ev.key) {
      case " ":
        ev.preventDefault();
        void send("Toggle");
        break;
      case "z":
      case "Z":
        void send("Previous");
        break;
      case "x":
      case "X":
        void send("Play");
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
        void send("Next");
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

// ── Drag & drop ─────────────────────────────────────────────────────────────

function bindDrop(): void {
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== "drop") return;
    const paths = event.payload.paths.filter((path) => {
      const ext = path.split(".").pop()?.toLowerCase() ?? "";
      return AUDIO_EXTENSIONS.includes(ext);
    });
    if (paths.length > 0) void loadAndPlay(paths);
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
      queue = payload.QueueChanged;
      render();
    } else if ("Warning" in payload) {
      toast(payload.Warning.message);
    } else if ("Fatal" in payload) {
      toast(payload.Fatal.message, "error");
    } else if ("TrackStarted" in payload) {
      toast(`▶ ${displayTitle(payload.TrackStarted)}`);
    }
  });

  bindSeek();
  bindSliders();
  bindControls();
  bindDrop();

  await send("QueryStatus");
  render();
  window.addEventListener("resize", updateMarquee);
}

void boot();
