import "./styles/global.scss";
import { mount } from "svelte";
import App from "./App.svelte";

/** The mounted root `App` component instance, per Svelte 5's `mount()` API. */
const app = mount(App, { target: document.getElementById("app")! });

export default app;
