/** Whether we're running inside a Tauri webview (vs a plain browser preview).
 *  Zero-import leaf module so both entries (main app and the presentation
 *  window) can share the probe without pulling each other's bundles. */
export function hasTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
