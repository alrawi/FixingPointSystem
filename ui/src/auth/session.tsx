// SPDX-License-Identifier: GPL-3.0-only
//
// Holds the API token of the signed in user and the client built from it.
// The token lives in sessionStorage, so it ends with the browser tab and never
// reaches other tabs or a later session.

import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { createApiClient, DEFAULT_API_BASE, type ApiClient } from "../api/client";

const STORAGE_KEY = "fps.token";

function readStoredToken(): string | null {
  try {
    return window.sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function storeToken(token: string | null): void {
  try {
    if (token === null) window.sessionStorage.removeItem(STORAGE_KEY);
    else window.sessionStorage.setItem(STORAGE_KEY, token);
  } catch {
    // Storage may be blocked. The session then lasts until the page reloads.
  }
}

/** The state that SessionProvider shares. */
export interface Session {
  baseUrl: string;
  token: string | null;
  /** The client for the current token, or null while signed out. */
  client: ApiClient | null;
  signIn: (token: string) => void;
  /** Signs out. A reason appears on the sign in page, for example after the token expired. */
  signOut: (reason?: string) => void;
  /** Why the last session ended, when it did not end by request. */
  signOutReason: string | null;
}

const SessionContext = createContext<Session | null>(null);

export interface SessionProviderProps {
  children: ReactNode;
  baseUrl?: string;
}

/** Provides the session to the component tree. */
export function SessionProvider({ children, baseUrl = DEFAULT_API_BASE }: SessionProviderProps) {
  const [token, setToken] = useState<string | null>(readStoredToken);
  const [signOutReason, setSignOutReason] = useState<string | null>(null);

  const signIn = useCallback((next: string) => {
    storeToken(next);
    setSignOutReason(null);
    setToken(next);
  }, []);

  const signOut = useCallback((reason?: string) => {
    storeToken(null);
    setSignOutReason(reason ?? null);
    setToken(null);
  }, []);

  const client = useMemo(
    () => (token === null ? null : createApiClient(token, { baseUrl })),
    [token, baseUrl],
  );

  const value = useMemo<Session>(
    () => ({ baseUrl, token, client, signIn, signOut, signOutReason }),
    [baseUrl, token, client, signIn, signOut, signOutReason],
  );
  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

/** Returns the session. Throws outside SessionProvider. */
export function useSession(): Session {
  const session = useContext(SessionContext);
  if (session === null) throw new Error("useSession needs a SessionProvider");
  return session;
}

/** Returns the client of the signed in session. Throws while signed out. */
export function useApiClient(): ApiClient {
  const { client } = useSession();
  if (client === null) throw new Error("useApiClient needs a signed in session");
  return client;
}
