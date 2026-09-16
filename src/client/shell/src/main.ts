import "./styles/global.scss";
import { mount } from "svelte";
import { theme, performanceController } from "@shadowcat/ui-kit";
import App from "./App.svelte";
import { readThemeMirror, readPerformanceMirror, writePerformanceMirror } from "./lib/sessionState.svelte";
import { readDeviceSignals } from "./lib/deviceSignals";

// Apply the last-used theme synchronously, before the app mounts, so pre-login
// screens (login, world select) honor it. `loadSessionState` later replaces it
// with the account's persisted value; an absent or garbage mirror resolves to
// the default theme inside `ThemeController.load`.
theme.load(readThemeMirror(localStorage));

// Per-device settings, resolved before mount so pre-login screens never flash the wrong
// budget (never the server ui_state — resolved fresh from this device's own signals).
performanceController.load(readPerformanceMirror(localStorage), readDeviceSignals());
performanceController.onChange = (p) => writePerformanceMirror(localStorage, p);

/** The mounted root `App` component instance, per Svelte 5's `mount()` API. */
const app = mount(App, { target: document.getElementById("app")! });

export default app;
