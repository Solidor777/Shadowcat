import "./styles/global.scss";
import { mount } from "svelte";
import { theme } from "@shadowcat/ui-kit";
import App from "./App.svelte";
import { readThemeMirror } from "./lib/sessionState.svelte";

// Apply the last-used theme synchronously, before the app mounts, so pre-login
// screens (login, world select) honor it. `loadSessionState` later replaces it
// with the account's persisted value; an absent or garbage mirror resolves to
// the default theme inside `ThemeController.load`.
theme.load(readThemeMirror(localStorage));

/** The mounted root `App` component instance, per Svelte 5's `mount()` API. */
const app = mount(App, { target: document.getElementById("app")! });

export default app;
