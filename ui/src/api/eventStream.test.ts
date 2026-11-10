// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it, vi } from "vitest";
import { EventStreamParser, readEventStream, type ServerEvent } from "./eventStream";

function streamOf(...chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  });
}

function sse(...chunks: string[]): Response {
  return new Response(streamOf(...chunks), {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

describe("EventStreamParser", () => {
  it("dispatches an event at the empty line", () => {
    const parser = new EventStreamParser();
    expect(parser.push('data: {"a":1}\n')).toEqual([]);
    expect(parser.push("\n")).toEqual([{ type: "message", data: '{"a":1}', lastEventId: "" }]);
  });

  it("joins data lines and keeps the event type and id", () => {
    const parser = new EventStreamParser();
    const events = parser.push("event: sample\nid: 7\ndata: one\ndata: two\n\n");
    expect(events).toEqual([{ type: "sample", data: "one\ntwo", lastEventId: "7" }]);
    expect(parser.currentLastEventId).toBe("7");
  });

  it("accepts CR, LF, and CRLF line ends, also when CRLF spans two chunks", () => {
    const parser = new EventStreamParser();
    const events = [
      ...parser.push("data: a\r\n\r"),
      ...parser.push("\ndata: b\r\r"),
      ...parser.push("data: c\n\n"),
    ];
    expect(events.map((event) => event.data)).toEqual(["a", "b", "c"]);
  });

  it("ignores comments, unknown fields, and an event without data", () => {
    const parser = new EventStreamParser();
    expect(parser.push(": keepalive\nfoo: bar\nevent: x\n\n")).toEqual([]);
    expect(parser.push("data: y\n\n")).toEqual([{ type: "message", data: "y", lastEventId: "" }]);
  });

  it("strips the byte order mark and only one leading space of a value", () => {
    const parser = new EventStreamParser();
    expect(parser.push("﻿data:  two spaces\n\n")[0]?.data).toBe(" two spaces");
  });

  it("reads a valid retry field and ignores an invalid one", () => {
    const parser = new EventStreamParser();
    parser.push("retry: 1500\n\n");
    expect(parser.retryMs).toBe(1500);
    parser.push("retry: soon\n\n");
    expect(parser.retryMs).toBe(1500);
  });

  it("ignores an id that contains NUL", () => {
    const parser = new EventStreamParser("3");
    parser.push("id: a\0b\ndata: x\n\n");
    expect(parser.currentLastEventId).toBe("3");
  });
});

describe("readEventStream", () => {
  it("sends the bearer token and reconnects with Last-Event-ID", async () => {
    const controller = new AbortController();
    const seen: Headers[] = [];
    const events: ServerEvent[] = [];
    const fetchMock = vi.fn((_url: RequestInfo | URL, init?: RequestInit) => {
      seen.push(new Headers(init?.headers));
      return Promise.resolve(
        seen.length === 1
          ? sse("retry: 5\nid: 1\ndata: first\n\n", "id: 2\ndata: second\n\n")
          : sse("id: 3\ndata: third\n\n"),
      );
    });
    const sleep = vi.fn((ms: number) => {
      expect(ms).toBe(5);
      if (seen.length === 2) controller.abort();
      return Promise.resolve();
    });

    const last = await readEventStream({
      url: "/v1/runs/r1/metrics:stream",
      token: "secret",
      onEvent: (event) => events.push(event),
      signal: controller.signal,
      fetch: fetchMock,
      sleep,
    });

    expect(events.map((event) => event.data)).toEqual(["first", "second", "third"]);
    expect(last).toBe("3");
    expect(seen[0]?.get("Authorization")).toBe("Bearer secret");
    expect(seen[0]?.get("Accept")).toBe("text/event-stream");
    expect(seen[0]?.has("Last-Event-ID")).toBe(false);
    expect(seen[1]?.get("Last-Event-ID")).toBe("2");
  });

  it("stops and reports an expired token", async () => {
    const onUnauthorized = vi.fn();
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 401 })));
    await readEventStream({
      url: "/stream",
      token: "expired",
      onEvent: () => undefined,
      onUnauthorized,
      fetch: fetchMock,
      sleep: () => Promise.resolve(),
    });
    expect(onUnauthorized).toHaveBeenCalledWith(401);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("gives up after the attempt limit when the server keeps failing", async () => {
    const onDisconnect = vi.fn();
    const fetchMock = vi.fn(() => Promise.resolve(new Response("busy", { status: 503 })));
    await readEventStream({
      url: "/stream",
      token: "t",
      onEvent: () => undefined,
      onDisconnect,
      maxAttempts: 3,
      fetch: fetchMock,
      sleep: () => Promise.resolve(),
    });
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(onDisconnect).toHaveBeenCalledTimes(3);
  });

  it("does not connect when the signal already aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    const fetchMock = vi.fn();
    await readEventStream({
      url: "/stream",
      token: "t",
      onEvent: () => undefined,
      signal: controller.signal,
      fetch: fetchMock,
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("stops without a retry on a status that the caller calls fatal", async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response("{}", { status: 501 })));
    const onFatalStatus = vi.fn();
    await readEventStream({
      url: "/stream",
      token: "t",
      onEvent: () => undefined,
      fetch: fetchMock,
      isFatalStatus: (status) => status === 501,
      onFatalStatus,
      sleep: () => Promise.resolve(),
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(onFatalStatus).toHaveBeenCalledWith(501);
  });
});
