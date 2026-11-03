// SPDX-License-Identifier: GPL-3.0-only
//
// A reader for `text/event-stream` responses over fetch(). The browser
// EventSource interface cannot send an Authorization header, so the UI reads
// GET /runs/{id}/metrics:stream with this module instead (decision D11).
// The parser follows the WHATWG HTML event stream interpretation rules.

/** One dispatched event of the stream. */
export interface ServerEvent {
  /** The event type, `message` when the stream named none. */
  type: string;
  data: string;
  /** The last event id of the stream at dispatch time, empty when none was set. */
  lastEventId: string;
}

/** Incremental parser that turns decoded text chunks into events. */
export class EventStreamParser {
  private buffer = "";
  private data: string[] = [];
  private type = "";
  private lastEventId = "";
  private started = false;
  private pendingCR = false;
  /** The newest valid `retry` value in milliseconds, or undefined. */
  retryMs: number | undefined;

  /** Restores the last event id, for example after a reconnect. */
  constructor(lastEventId = "") {
    this.lastEventId = lastEventId;
  }

  get currentLastEventId(): string {
    return this.lastEventId;
  }

  /** Feeds a chunk and returns the events that it completed. */
  push(chunk: string): ServerEvent[] {
    let text = chunk;
    if (text === "") return [];
    if (!this.started) {
      this.started = true;
      if (text.startsWith("﻿")) text = text.slice(1);
    }
    // A CR at the end of the previous chunk already ended a line, so a LF that
    // opens this chunk belongs to the same line break.
    if (this.pendingCR && text.startsWith("\n")) text = text.slice(1);
    this.pendingCR = text.endsWith("\r");
    this.buffer += text;

    const events: ServerEvent[] = [];
    const lineBreak = /\r\n|\r|\n/g;
    let start = 0;
    let match: RegExpExecArray | null;
    while ((match = lineBreak.exec(this.buffer)) !== null) {
      const line = this.buffer.slice(start, match.index);
      start = match.index + match[0].length;
      const event = this.line(line);
      if (event) events.push(event);
    }
    this.buffer = this.buffer.slice(start);
    return events;
  }

  private line(line: string): ServerEvent | undefined {
    if (line === "") return this.dispatch();
    if (line.startsWith(":")) return undefined;
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    switch (field) {
      case "data":
        this.data.push(value);
        break;
      case "event":
        this.type = value;
        break;
      case "id":
        if (!value.includes("\0")) this.lastEventId = value;
        break;
      case "retry":
        if (/^[0-9]+$/.test(value)) this.retryMs = Number(value);
        break;
      default:
        break;
    }
    return undefined;
  }

  private dispatch(): ServerEvent | undefined {
    const type = this.type || "message";
    this.type = "";
    if (this.data.length === 0) return undefined;
    const data = this.data.join("\n");
    this.data = [];
    return { type, data, lastEventId: this.lastEventId };
  }
}

/** Options of readEventStream. */
export interface ReadEventStreamOptions {
  url: string;
  token: string;
  onEvent: (event: ServerEvent) => void;
  /** Called once when the controller rejects the token with 401 or 403. Reading stops. */
  onUnauthorized?: (status: number) => void;
  /** Called after each lost connection, before the reader waits to reconnect. */
  onDisconnect?: (reason: unknown) => void;
  signal?: AbortSignal;
  /** Initial reconnect delay in milliseconds. A `retry` field of the stream replaces it. */
  retryMs?: number;
  /** Stops after this many consecutive failed connection attempts. Unlimited by default. */
  maxAttempts?: number;
  lastEventId?: string;
  fetch?: typeof globalThis.fetch;
  /** Replacement for the reconnect timer, used by tests. */
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

function defaultSleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

/**
 * Reads an event stream until the signal aborts, the token is rejected, or the
 * attempt limit is reached. Each reconnect sends the Last-Event-ID header.
 * Resolves with the last event id that the stream set.
 */
export async function readEventStream(options: ReadEventStreamOptions): Promise<string> {
  const doFetch = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
  const sleep = options.sleep ?? defaultSleep;
  let retryMs = options.retryMs ?? 3000;
  let lastEventId = options.lastEventId ?? "";
  let failures = 0;
  const aborted = () => options.signal?.aborted === true;

  while (!aborted()) {
    const headers: Record<string, string> = {
      Accept: "text/event-stream",
      Authorization: `Bearer ${options.token}`,
    };
    if (lastEventId !== "") headers["Last-Event-ID"] = lastEventId;

    const parser = new EventStreamParser(lastEventId);
    let received = false;
    try {
      const init: RequestInit = { headers, cache: "no-store" };
      if (options.signal) init.signal = options.signal;
      const response = await doFetch(options.url, init);
      if (response.status === 401 || response.status === 403) {
        options.onUnauthorized?.(response.status);
        return lastEventId;
      }
      if (!response.ok || response.body === null) {
        throw new Error(`The event stream answered with HTTP ${response.status}.`);
      }
      const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        for (const event of parser.push(value)) {
          received = true;
          options.onEvent(event);
        }
        lastEventId = parser.currentLastEventId;
        if (parser.retryMs !== undefined) retryMs = parser.retryMs;
      }
      options.onDisconnect?.(new Error("The event stream ended."));
    } catch (error) {
      if (aborted()) break;
      options.onDisconnect?.(error);
    }
    failures = received ? 0 : failures + 1;
    if (options.maxAttempts !== undefined && failures >= options.maxAttempts) break;
    await sleep(retryMs, options.signal);
  }
  return lastEventId;
}
