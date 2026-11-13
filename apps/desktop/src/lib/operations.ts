import type { LiveOperation, OperationEvent, OperationPrompt, OperationState, RecipeFailure, UUID } from "../types";

/** Log lines kept per operation; older lines are dropped. */
export const MAX_LOG_LINES = 2000;
const MAX_TIMELINE_EVENTS = 50;

export const TERMINAL_STATES: ReadonlySet<OperationState> = new Set(["succeeded", "failed", "cancelled", "interrupted"]);
export const LIVE_STATES: ReadonlySet<OperationState> = new Set(["planned", "preflight", "running", "waiting_for_user", "cancelling"]);

export function isTerminal(state: OperationState | undefined): boolean {
  return state !== undefined && TERMINAL_STATES.has(state);
}

/** Everything the drawer renders for one operation, maintained incrementally per event. */
export interface OperationEntry {
  lastSequence: number;
  state: OperationState;
  latest: OperationEvent | null;
  timeline: OperationEvent[];
  logs: string[];
  failures: Map<string, RecipeFailure>;
  prompt: OperationPrompt | null;
}

export type OperationStore = ReadonlyMap<UUID, OperationEntry>;

function emptyEntry(): OperationEntry {
  return { lastSequence: 0, state: "planned", latest: null, timeline: [], logs: [], failures: new Map(), prompt: null };
}

function appendCapped<T>(items: T[], item: T, limit: number): T[] {
  return items.length >= limit ? [...items.slice(items.length - limit + 1), item] : [...items, item];
}

/**
 * Applies one backend event with bounded work: duplicates and stale events are dropped by
 * sequence, logs and the timeline are capped, and finished operations other than `keepId` are
 * pruned.
 * Returns the same store when nothing changed so React can skip the render.
 */
export function applyOperationEvent(store: OperationStore, event: OperationEvent, keepId: UUID | null): OperationStore {
  const current = store.get(event.operation_id) ?? emptyEntry();
  if (event.sequence <= current.lastSequence) return store;
  // Never mutate `current`: React may run this updater twice (StrictMode) and keeps the old
  // state for comparison. Copies are bounded by the caps, so each event stays O(1).
  const next: OperationEntry = { ...current, lastSequence: event.sequence, state: event.state };
  if (event.log_line) {
    next.logs = appendCapped(current.logs, event.log_line, MAX_LOG_LINES);
  } else {
    next.latest = event;
    next.timeline = appendCapped(current.timeline, event, MAX_TIMELINE_EVENTS);
    next.prompt = event.state === "waiting_for_user" ? event.prompt ?? current.prompt : null;
  }
  if (event.failure) next.failures = new Map(current.failures).set(event.failure.recipe_id, event.failure);
  const updated = new Map(store);
  if (isTerminal(event.state) && event.operation_id !== keepId) updated.delete(event.operation_id);
  else updated.set(event.operation_id, next);
  return updated;
}

export function entryFromEvents(events: OperationEvent[]): OperationEntry {
  let store: OperationStore = new Map();
  const id = events[0]?.operation_id ?? "";
  for (const event of events) store = applyOperationEvent(store, event, id);
  return store.get(id) ?? emptyEntry();
}

/** Re-attaches to operations the backend still runs (after a webview reload). */
export function entryFromLive(live: LiveOperation): OperationEntry {
  const entry = emptyEntry();
  entry.state = live.state;
  entry.prompt = live.prompt;
  if (live.latest) {
    entry.lastSequence = live.latest.sequence;
    entry.latest = { ...live.latest, prompt: live.prompt ?? live.latest.prompt };
    entry.timeline.push(entry.latest);
  }
  return entry;
}
