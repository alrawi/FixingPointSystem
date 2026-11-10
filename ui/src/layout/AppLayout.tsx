// SPDX-License-Identifier: GPL-3.0-only

import { Button } from "react-aria-components";
import { NavLink, Outlet } from "react-router";
import { useSession } from "../auth/session";

/** Navigation entries. Later UI packages add their views here. */
const NAV_ITEMS = [
  { to: "/", label: "Capabilities", end: true },
  { to: "/modules", label: "Modules", end: false },
  { to: "/configs", label: "Configurations", end: false },
] as const;

/**
 * The page frame after sign in: a skip link, a banner with the product name
 * and the sign out button, the main navigation, and the main region.
 */
export function AppLayout() {
  const { signOut } = useSession();
  return (
    <div className="app">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="banner">
        <span className="product">FixingPointSystem</span>
        <Button
          className="sign-out"
          onPress={() => {
            signOut();
          }}
        >
          Sign out
        </Button>
      </header>
      <nav className="nav" aria-label="Main">
        <ul>
          {NAV_ITEMS.map((item) => (
            <li key={item.to}>
              <NavLink to={item.to} end={item.end}>
                {item.label}
              </NavLink>
            </li>
          ))}
        </ul>
      </nav>
      <main className="content" id="main" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}
