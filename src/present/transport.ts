import { emit, listen } from "@tauri-apps/api/event";
import { hasTauri } from "../env";
import { PRESENT_CHANNEL, type PresentState } from "./channel";

// Cross-window transport for the presentation window. In the desktop app the
// main window and each present window are separate WebView2 instances, so a
// BroadcastChannel does NOT reach across them — we use the Tauri event bus
// (routed through the Rust core) instead. In a plain browser (dev preview) we
// fall back to BroadcastChannel. The Tauri imports are inert until called, and
// every call is gated by hasTauri().

const STATE_EVENT = "present-state";
const HELLO_EVENT = "present-hello";

function tauriEmit(event: string, payload?: unknown): void {
  emit(event, payload ?? {}).catch((e) => console.error("present emit failed", e));
}

function tauriListen(event: string, cb: (payload: unknown) => void): () => void {
  let un: (() => void) | undefined;
  let cancelled = false;
  listen(event, (e) => cb(e.payload))
    .then((fn) => {
      if (cancelled) fn();
      else un = fn;
    })
    .catch((e) => console.error("present listen failed", e));
  return () => {
    cancelled = true;
    un?.();
  };
}

// --- Browser BroadcastChannel (single shared instance) ---
let channel: BroadcastChannel | null = null;
function getChannel(): BroadcastChannel {
  if (!channel) channel = new BroadcastChannel(PRESENT_CHANNEL);
  return channel;
}

/** Push the latest transcript state to any open present windows. */
export function postPresentState(state: PresentState): void {
  if (hasTauri()) tauriEmit(STATE_EVENT, state);
  else getChannel().postMessage(state);
}

/** A present window announces it just opened and wants the current state. */
export function postPresentHello(): void {
  if (hasTauri()) tauriEmit(HELLO_EVENT);
  else getChannel().postMessage({ type: "hello" });
}

/** Subscribe to state pushes (present window side). Returns an unsubscribe fn. */
export function onPresentState(cb: (s: PresentState) => void): () => void {
  if (hasTauri()) return tauriListen(STATE_EVENT, (p) => cb(p as PresentState));
  const ch = getChannel();
  const handler = (e: MessageEvent) => {
    if (e.data?.type === "state") cb(e.data as PresentState);
  };
  ch.addEventListener("message", handler);
  return () => ch.removeEventListener("message", handler);
}

/** Subscribe to "hello" requests (broadcaster side). Returns an unsubscribe fn. */
export function onPresentHello(cb: () => void): () => void {
  if (hasTauri()) return tauriListen(HELLO_EVENT, () => cb());
  const ch = getChannel();
  const handler = (e: MessageEvent) => {
    if (e.data?.type === "hello") cb();
  };
  ch.addEventListener("message", handler);
  return () => ch.removeEventListener("message", handler);
}
