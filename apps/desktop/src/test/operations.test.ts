import { describe, expect, it } from "vitest";

import { applyOperationEvent, MAX_LOG_LINES, type OperationStore } from "../lib/operations";
import type { OperationEvent } from "../types";

function event(operation_id: string, sequence: number, overrides: Partial<OperationEvent> = {}): OperationEvent {
  return { operation_id, sequence, state: "running", step: 1, total_steps: 2, recipe_id: null, title: "Downloading", detail: null, progress: 0.5, prompt: null, log_line: null, failure: null, timestamp: "2026-11-12T00:00:00Z", ...overrides };
}

describe("operation event store", () => {
  it("drops duplicate and stale events by sequence and caps the log", () => {
    let store: OperationStore = new Map();
    const started = performance.now();
    for (let sequence = 1; sequence <= 40_000; sequence += 1) {
      store = applyOperationEvent(store, event("a", sequence, { log_line: `line ${sequence}` }), "a");
    }
    // 40 000 events must not degrade quadratically.
    expect(performance.now() - started).toBeLessThan(2_000);
    const entry = store.get("a")!;
    expect(entry.logs.length).toBe(MAX_LOG_LINES);
    expect(entry.logs.at(-1)).toBe("line 40000");
    expect(applyOperationEvent(store, event("a", 40_000), "a")).toBe(store);
    expect(applyOperationEvent(store, event("a", 12), "a")).toBe(store);
  });

  it("never mutates the previous state", () => {
    const before = applyOperationEvent(new Map(), event("a", 1, { log_line: "one" }), "a");
    const entry = before.get("a")!;
    const after = applyOperationEvent(before, event("a", 2, { log_line: "two", failure: { recipe_id: "r", recipe_title: "R", kind: "failed", message: "m" } }), "a");
    expect(entry.logs).toEqual(["one"]);
    expect(entry.failures.size).toBe(0);
    expect(after.get("a")!.logs).toEqual(["one", "two"]);
  });

  it("tracks the prompt and forgets finished operations that are not on screen", () => {
    let store: OperationStore = new Map();
    const prompt = { id: "p", level: "confirmation" as const, title: "Go?", message: "m", choices: [] };
    store = applyOperationEvent(store, event("a", 1, { state: "waiting_for_user", prompt }), "a");
    expect(store.get("a")?.prompt?.id).toBe("p");
    store = applyOperationEvent(store, event("a", 2, { state: "running" }), "a");
    expect(store.get("a")?.prompt).toBeNull();
    store = applyOperationEvent(store, event("b", 1), "a");
    store = applyOperationEvent(store, event("b", 2, { state: "succeeded" }), "a");
    expect(store.has("b")).toBe(false);
    store = applyOperationEvent(store, event("a", 3, { state: "interrupted" }), "a");
    expect(store.get("a")?.state).toBe("interrupted");
  });
});
