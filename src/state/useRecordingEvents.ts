import { useEffect } from "react";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { Backend } from "../backend";
import type { Conversation, Message, PendingSegment } from "../types";
import { logErr } from "./helpers";

interface Setters {
  setMessages: Dispatch<SetStateAction<Record<string, Message[]>>>;
  setPending: Dispatch<SetStateAction<PendingSegment[]>>;
  setConversations: Dispatch<SetStateAction<Conversation[]>>;
  setError: Dispatch<SetStateAction<string | null>>;
  /** Always-current recording flag — placeholder events that arrive after a
   *  stop (queued in the Tauri event loop) are ignored so they can't re-raise a
   *  bar that the stop already cleared. */
  recordingRef: MutableRefObject<boolean>;
}

/** Race-safe unsubscribe plumbing for a backend event subscription: if the
 *  effect is torn down before the subscribe promise resolves, the listener is
 *  released the moment it arrives instead of leaking. Returns the cleanup. */
function subscribe(sub: Promise<() => void>, tag: string): () => void {
  let unlisten: (() => void) | undefined;
  let cancelled = false;
  sub
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch(logErr(tag));
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/** Subscribe to the Rust recording pipeline's live events — transcript rows,
 *  non-fatal errors, and the §10.8 placeholder lifecycle — wiring each into the
 *  provided state setters. Extracted from AppProvider to keep it readable. */
export function useRecordingEvents(backend: Backend, s: Setters): void {
  const { setMessages, setPending, setConversations, setError, recordingRef } = s;

  // Live transcript messages emitted by the Rust recording pipeline.
  useEffect(
    () =>
      subscribe(
        backend.onTranscriptMessage((m, pendingId) => {
          setMessages((prev) => {
            const list = prev[m.conversationId] ?? [];
            const i = list.findIndex((x) => x.id === m.id);
            const next = i >= 0 ? list.map((x) => (x.id === m.id ? m : x)) : [...list, m];
            return { ...prev, [m.conversationId]: next };
          });
          // The real row replaces its in-flight placeholder (§10.8).
          if (pendingId !== undefined) {
            setPending((prev) => prev.filter((p) => p.pendingId !== pendingId));
          }
          setConversations((prev) =>
            prev.map((c) =>
              c.id === m.conversationId ? { ...c, updatedAt: m.createdAt } : c
            )
          );
        }),
        "transcript subscription failed"
      ),
    [backend, setMessages, setPending, setConversations]
  );

  // Non-fatal recording errors (STT/translation), shown as a dismissible toast.
  useEffect(
    () =>
      subscribe(
        backend.onRecordingError((message) => setError(message)),
        "error subscription failed"
      ),
    [backend, setError]
  );

  // Live placeholders (§10.8). A pause raises a "silence" bar that fills over
  // `hangoverMs`; if the speaker resumes it's cancelled, otherwise it flips to
  // a "processing" shimmer until the real message (or a cancel) arrives.
  useEffect(() => {
    // Insert or update the placeholder for `pendingId`, restarting the phase
    // clock so the CSS bar animates from the new phase's start. Ignored once
    // recording has stopped — a late silence/pending event must not re-raise a
    // bar that the stop already cleared.
    const upsert = (next: PendingSegment) => {
      if (!recordingRef.current) return;
      setPending((prev) => {
        const i = prev.findIndex((p) => p.pendingId === next.pendingId);
        if (i < 0) return [...prev, next];
        const copy = prev.slice();
        copy[i] = next;
        return copy;
      });
    };
    const stops = [
      subscribe(
        backend.onSegmentSilence((p) => {
          upsert({
            pendingId: p.pendingId,
            conversationId: p.conversationId,
            phase: "silence",
            hangoverMs: p.hangoverMs,
            since: Date.now(),
          });
        }),
        "segment-silence subscription failed"
      ),
      subscribe(
        backend.onSegmentPending((p) => {
          upsert({
            pendingId: p.pendingId,
            conversationId: p.conversationId,
            phase: "processing",
            since: Date.now(),
          });
        }),
        "segment-pending subscription failed"
      ),
      subscribe(
        backend.onSegmentCancelled((pendingId) => {
          setPending((prev) => prev.filter((p) => p.pendingId !== pendingId));
        }),
        "segment-cancelled subscription failed"
      ),
    ];
    return () => stops.forEach((stop) => stop());
  }, [backend, setPending, recordingRef]);
}
