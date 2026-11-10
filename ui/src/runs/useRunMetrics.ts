// SPDX-License-Identifier: GPL-3.0-only
//
// Follows the metrics of one run. The hook reads GET /runs/{id}/metrics:stream
// with the fetch based reader, because EventSource cannot send the bearer
// token. It seeds the store from GET /runs/{id}/metrics so a view opened in
// the middle of a run has history, fills the gap after a dropped connection
// the same way, and polls that endpoint once per second when the controller
// answers the stream with 501, as `fps watch` does.

import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  ApiError,
  fetchRunMetrics,
  isFinalRunState,
  type MetricSample,
  type Run,
} from "../api/client";
import { readEventStream } from "../api/eventStream";
import { useSession } from "../auth/session";
import { MetricsStore, RUN_VIEW } from "./metricsStore";

/** Timings of the run view. Tests shorten them. */
export interface RunSettings {
  /** How long the view waits for the rest of a tick before it redraws. */
  flushMs: number;
  /** The interval of the run state poll and of the metrics poll fallback. */
  pollMs: number;
}

export const DEFAULT_RUN_SETTINGS: RunSettings = { flushMs: 100, pollMs: 1000 };

export const RunSettingsContext = createContext<RunSettings>(DEFAULT_RUN_SETTINGS);

export function useRunSettings(): RunSettings {
  return useContext(RunSettingsContext);
}

/** How the view receives metrics at the moment. */
export type FeedStatus = "connecting" | "live" | "reconnecting" | "polling" | "ended" | "failed";

export interface RunMetrics {
  store: MetricsStore;
  status: FeedStatus;
  /** Why the feed failed, when it did. */
  error: string | undefined;
}

/** A status that retrying the stream cannot change: 501 means polling, any other client error fails. */
function isFatalStatus(status: number): boolean {
  return status === 501 || (status >= 400 && status < 500 && status !== 408 && status !== 429);
}

/** Resolves after a delay, or at once when the signal aborts. */
function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
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
 * Follows the metrics of a run until it ends. `run` is the polled state of the run: the feed
 * stops after the run reaches a final state and the last samples arrived.
 */
export function useRunMetrics(runId: string, run: Run | undefined): RunMetrics {
  const { client, token, baseUrl, signOut } = useSession();
  const settings = useRunSettings();
  // The page keys this hook's component by run id, so one store serves one run.
  const [store] = useState(() => new MetricsStore());
  // A redraw of the store renders the page that uses this hook, and the page renders its children.
  useSyncExternalStore(store.subscribe, store.getSnapshot);
  const [status, setStatus] = useState<FeedStatus>("connecting");
  const [error, setError] = useState<string | undefined>(undefined);
  // The feed reads the newest run state through a ref, so a state change never restarts it.
  const finalRef = useRef(false);
  const final = run === undefined ? false : isFinalRunState(run.state);
  useEffect(() => {
    finalRef.current = final;
  }, [final]);

  useEffect(() => {
    if (client === null || token === null) return;
    const abort = new AbortController();
    const { signal } = abort;
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleFlush = () => {
      flushTimer ??= setTimeout(() => {
        flushTimer = undefined;
        store.flush();
      }, settings.flushMs);
    };
    const flushNow = () => {
      if (flushTimer !== undefined) clearTimeout(flushTimer);
      flushTimer = undefined;
      store.flush();
    };
    const ingest = (samples: readonly MetricSample[]) => {
      store.add(samples);
      scheduleFlush();
    };
    const fail = (reason: unknown) => {
      if (signal.aborted) return;
      if (reason instanceof ApiError && reason.isUnauthorized) {
        signOut("The controller no longer accepts the token. Sign in again.");
        return;
      }
      setError(reason instanceof Error ? reason.message : "The metrics could not be read.");
      setStatus("failed");
    };
    /** Reads the samples since the newest tick that the store holds. */
    const backfill = async () => {
      const from = store.newest();
      const samples = await fetchRunMetrics(client, runId, {
        view: RUN_VIEW,
        ...(from === undefined ? {} : { from }),
        signal,
      });
      ingest(samples);
    };

    const pollLoop = async () => {
      setStatus("polling");
      for (;;) {
        // Read the state before the poll, so the poll after the end holds the last samples.
        const wasFinal = finalRef.current;
        try {
          await backfill();
        } catch (reason) {
          if (signal.aborted) return;
          if (reason instanceof ApiError && reason.status < 500 && reason.status !== 408) {
            fail(reason);
            return;
          }
        }
        if (wasFinal) {
          flushNow();
          setStatus("ended");
          return;
        }
        await pause(settings.pollMs, signal);
        if (signal.aborted) return;
      }
    };

    const streamUrl = `${baseUrl.replace(/\/$/, "")}/runs/${encodeURIComponent(runId)}/metrics:stream?view=${RUN_VIEW}&side=client`;
    let needBackfill = false;
    const stream = async () => {
      try {
        await backfill();
      } catch (reason) {
        // Missing history only shortens the charts, but a rejected token or a missing run ends the view.
        if (reason instanceof ApiError && (reason.isUnauthorized || reason.status === 404)) {
          fail(reason);
          return;
        }
      }
      if (signal.aborted) return;
      let ended = false;
      const outcome = { fellBack: false };
      await readEventStream({
        url: streamUrl,
        token,
        signal,
        retryMs: settings.pollMs,
        isFatalStatus,
        onUnauthorized: () => {
          signOut("The controller no longer accepts the token. Sign in again.");
        },
        onFatalStatus: (code) => {
          if (code === 501) outcome.fellBack = true;
          else fail(new ApiError(code, undefined));
        },
        onDisconnect: () => {
          if (signal.aborted || ended) return;
          needBackfill = true;
          setStatus("reconnecting");
          if (finalRef.current) {
            // The run ended while the connection was down, so no end event will come.
            ended = true;
            void backfill()
              .catch(() => undefined)
              .finally(() => {
                flushNow();
                setStatus("ended");
                abort.abort();
              });
          }
        },
        onEvent: (event) => {
          if (event.type === "end") {
            ended = true;
            flushNow();
            setStatus("ended");
            abort.abort();
            return;
          }
          if (event.type !== "message") return;
          let sample: MetricSample;
          try {
            sample = JSON.parse(event.data) as MetricSample;
          } catch {
            return;
          }
          if (needBackfill) {
            needBackfill = false;
            // The stream carries only the newest tick, so fetch the ticks of the gap.
            void backfill().catch(() => undefined);
          }
          setStatus("live");
          ingest([sample]);
        },
      });
      if (outcome.fellBack && !abort.signal.aborted) await pollLoop();
    };

    void stream().catch(fail);
    return () => {
      abort.abort();
      if (flushTimer !== undefined) clearTimeout(flushTimer);
    };
    // The settings are fixed for the life of the view.
  }, [client, token, baseUrl, runId, store, settings, signOut]);

  return { store, status, error };
}
