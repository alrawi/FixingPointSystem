// SPDX-License-Identifier: GPL-3.0-only

import { useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { ApiError, createApiClient, fetchCapabilities } from "../api/client";
import { capabilitiesKey } from "../pages/capabilitiesQuery";
import { useSession } from "./session";

/** Asks for an API token and checks it against GET /capabilities before signing in. */
export function SignInPage() {
  const { baseUrl, signIn, signOutReason } = useSession();
  const queryClient = useQueryClient();
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const errorId = useId();

  async function onSubmit(event: { preventDefault: () => void }) {
    event.preventDefault();
    const candidate = token.trim();
    if (candidate === "") return;
    setPending(true);
    setError(null);
    try {
      const capabilities = await fetchCapabilities(createApiClient(candidate, { baseUrl }));
      queryClient.setQueryData(capabilitiesKey, capabilities);
      signIn(candidate);
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.isUnauthorized
          ? "The controller rejected this token. Check the token and its read scope."
          : caught instanceof ApiError
            ? caught.message
            : "The controller is not reachable.",
      );
      setPending(false);
    }
  }

  const notice = error ?? signOutReason;
  return (
    <main className="sign-in" id="main">
      <h1>Sign in to FixingPointSystem</h1>
      <Form
        onSubmit={(event) => void onSubmit(event)}
        aria-describedby={notice === null ? undefined : errorId}
      >
        <TextField
          name="token"
          type="password"
          isRequired
          autoComplete="off"
          value={token}
          onChange={setToken}
          isDisabled={pending}
        >
          <Label>API token</Label>
          <Input />
          <Text slot="description">A controller token with at least the read scope.</Text>
          <FieldError>Enter an API token.</FieldError>
        </TextField>
        {notice !== null && (
          <p className="notice" role="alert" id={errorId}>
            {notice}
          </p>
        )}
        <Button type="submit" isPending={pending}>
          {pending ? "Signing in" : "Sign in"}
        </Button>
      </Form>
    </main>
  );
}
