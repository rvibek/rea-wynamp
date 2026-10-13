import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

interface PathsSnapshot {
  data_dir: string;
  config_dir: string;
  cache_dir: string;
}

async function boot(): Promise<void> {
  const status = document.querySelector<HTMLSpanElement>("#status");
  try {
    const version = await invoke<string>("app_version");
    if (status) status.textContent = `v${version} · phase 1`;

    const paths = await invoke<PathsSnapshot>("app_paths");
    const list = document.querySelector<HTMLDListElement>("#paths");
    if (list) {
      list.hidden = false;
      setText("#path-data", paths.data_dir);
      setText("#path-config", paths.config_dir);
      setText("#path-cache", paths.cache_dir);
    }
  } catch (error) {
    if (status) status.textContent = "bridge error";
    console.error("Tauri bridge error:", error);
  }
}

function setText(selector: string, value: string): void {
  const el = document.querySelector<HTMLElement>(selector);
  if (el) el.textContent = value;
}

void boot();
