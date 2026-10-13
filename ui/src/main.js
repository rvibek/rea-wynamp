import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
async function boot() {
    const status = document.querySelector("#status");
    try {
        const version = await invoke("app_version");
        if (status)
            status.textContent = `v${version} · phase 1`;
        const paths = await invoke("app_paths");
        const list = document.querySelector("#paths");
        if (list) {
            list.hidden = false;
            setText("#path-data", paths.data_dir);
            setText("#path-config", paths.config_dir);
            setText("#path-cache", paths.cache_dir);
        }
    }
    catch (error) {
        if (status)
            status.textContent = "bridge error";
        console.error("Tauri bridge error:", error);
    }
}
function setText(selector, value) {
    const el = document.querySelector(selector);
    if (el)
        el.textContent = value;
}
void boot();
