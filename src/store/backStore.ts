import { useEffect, useRef } from "react";
import { create } from "zustand";
import { useNavStore } from "./navStore";

/** A screen's own "go back" action.
 *
 *  Most back arrows in the app close something held in local state -- a
 *  statement drill-down, a transaction detail -- which nothing outside that
 *  component can see. A gesture, or Android's hardware back button, needs to
 *  reach them, so each registers here while it is open and the most recently
 *  opened one wins. */
interface Entry {
  id: number;
  run: () => void;
}

interface BackState {
  stack: Entry[];
  push: (run: () => void) => number;
  remove: (id: number) => void;
}

let nextId = 1;

const useBackStore = create<BackState>((set) => ({
  stack: [],
  push: (run) => {
    const id = nextId++;
    set((s) => ({ stack: [...s.stack, { id, run }] }));
    return id;
  },
  remove: (id) => set((s) => ({ stack: s.stack.filter((e) => e.id !== id) })),
}));

/** Register `run` as the back action while `active`.
 *
 *  The callback is kept in a ref so that a handler re-created on every render
 *  -- which is every inline arrow function -- does not churn the stack. */
export function useBackHandler(active: boolean, run: () => void): void {
  const push = useBackStore((s) => s.push);
  const remove = useBackStore((s) => s.remove);
  const latest = useRef(run);
  latest.current = run;

  useEffect(() => {
    if (!active) return;
    const id = push(() => latest.current());
    return () => remove(id);
  }, [active, push, remove]);
}

/** True when something is open that back would close. */
export function canGoBack(): boolean {
  return (
    useBackStore.getState().stack.length > 0 ||
    useNavStore.getState().history.length > 0
  );
}

/** Notify `listener` whenever what back would do changes -- something opened
 *  or closed. Returns the unsubscribe function. */
export function onBackStackChange(listener: () => void): () => void {
  const a = useBackStore.subscribe(listener);
  const b = useNavStore.subscribe(listener);
  return () => {
    a();
    b();
  };
}

/** Run the innermost back action. Returns false when there was nothing to do,
 *  so a caller can fall through to whatever it would otherwise have done. */
export function goBack(): boolean {
  const { stack } = useBackStore.getState();
  const top = stack[stack.length - 1];
  if (top) {
    top.run();
    return true;
  }
  if (useNavStore.getState().history.length > 0) {
    useNavStore.getState().goBack();
    return true;
  }
  return false;
}
