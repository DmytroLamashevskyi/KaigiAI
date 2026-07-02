import { useEffect, useRef, type DependencyList, type RefObject } from "react";

/** Ref for a scrollable transcript body that snaps to the bottom whenever any
 *  of `deps` changes. Pass the counts that signal new content — keep them as
 *  SEPARATE deps (e.g. `[messages.length, pending.length]`), not a sum: when a
 *  pending row finalizes into a message the sum stays equal but both slots
 *  change, and the scroll must still fire. */
export function useAutoScroll<T extends HTMLElement>(deps: DependencyList): RefObject<T> {
  const ref = useRef<T>(null);
  useEffect(() => {
    ref.current?.scrollTo({ top: ref.current.scrollHeight });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return ref;
}
