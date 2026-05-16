import { EventEmitter } from "node:events";
import type { Event } from "./types.js";

export interface MountOptions {
  /** Executable name or path. Defaults to "camouflage-tui" (PATH lookup). */
  bin?: string;
  /** Extra args to pass to the renderer. `--stdin-events --emit-responses`
   *  are always appended by the binding. */
  args?: string[];
  /** Environment overrides for the child. Merged with process.env. */
  env?: NodeJS.ProcessEnv;
  /** If true (default), renderer stderr is forwarded to Node's stderr.
   *  If false, captured and exposed via the "stderr" event. */
  inheritStderr?: boolean;
}

export interface PermissionResponseEvent {
  request_id: string;
  choice: "allow_once" | "allow_session" | "deny";
  feedback?: string;
}

export interface InvalidEvent {
  line: string;
  error: string;
}

export interface ExitEvent {
  code: number | null;
  signal: NodeJS.Signals | null;
}

export interface CamouflageHandle extends EventEmitter {
  /** Emit one event into the renderer. */
  emit(event_type: string, payload?: object): boolean;
  /** Emit a pre-built Event object. */
  emitEvent(ev: { event_type: string; payload?: object }): boolean;
  /** Gracefully close: end stdin, wait for child exit, resolve with code. */
  close(): Promise<number>;
  /** Force-kill the renderer. */
  kill(signal?: NodeJS.Signals): void;

  // Typed event subscriptions (EventEmitter overrides):
  on(event: "userInput", listener: (text: string) => void): this;
  on(event: "permissionResponse", listener: (resp: PermissionResponseEvent) => void): this;
  on(event: "event", listener: (ev: Event) => void): this;
  on(event: "invalid", listener: (info: InvalidEvent) => void): this;
  on(event: "stderr", listener: (chunk: string) => void): this;
  on(event: "exit", listener: (info: ExitEvent) => void): this;
  on(event: string, listener: (...args: any[]) => void): this;
}

/**
 * Spawn the Camouflage renderer and return a handle for emitting events
 * and listening to renderer→host outbound events.
 *
 * @example
 *   import { mount } from "camouflage";
 *   const cam = await mount();
 *   cam.emit("SessionStarted", {});
 *   cam.on("userInput", (text) => console.log("got input:", text));
 *   // ... when done:
 *   await cam.close();
 */
export function mount(opts?: MountOptions): Promise<CamouflageHandle>;
