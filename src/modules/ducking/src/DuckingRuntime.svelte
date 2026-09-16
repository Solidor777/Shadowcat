<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { consoleLogger } from "@shadowcat/core";
  import type { DuckSourcesController } from "./controller";
  import { MicVadSource } from "./micVad";
  import { readDuckingMirror } from "./duckingMirror";

  // No visible output — this component's only job is wiring `DuckSourcesController` to the
  // real `AppContext.audio.duck`. Contributed into `shadowcat.surface:overlay` (the same
  // always-mounted-for-the-session surface `AssetPickOverlay` uses) rather than
  // `SETTINGS_SECTION_CONTRACT`, because `register(ctx)`'s `ModuleContext` carries no `audio`
  // member (that lives only on the Svelte-reachable `AppContext`) and the key/OS-monitor
  // sources must keep ducking while the Settings panel — which unmounts on close — is closed.

  let {
    controller,
  }: {
    /** The shared controller constructed once in `register(ctx)`. */
    controller: DuckSourcesController;
  } = $props();

  const ctx = getAppContext();

  let mic: MicVadSource | null = null;

  onMount(() => {
    const keySink = ctx.audio.duck.addSource("ducking:key");
    const osSink = ctx.audio.duck.addSource("ducking:os-monitor");
    controller.wireToAudioDuck(keySink, osSink);

    controller.micToggle = async (enabled: boolean) => {
      if (!enabled) {
        mic?.disable();
        return null;
      }
      const audioContext = ctx.audio.context();
      if (!audioContext) return "unknown"; // not yet unlocked — the statusbar's "Enable
                                            // audio" control must run first
      if (!mic) {
        const prefs = readDuckingMirror(localStorage);
        mic = new MicVadSource({
          audioContext,
          workletUrl: new URL("./vad.worklet.ts", import.meta.url),
          sensitivity: prefs.micSensitivity,
          logger: consoleLogger(),
        });
        mic.setSink(ctx.audio.duck.addSource("ducking:mic"));
      }
      return mic.enable();
    };
  });

  onDestroy(() => {
    controller.micToggle = null;
    ctx.audio.duck.removeSource("ducking:key");
    ctx.audio.duck.removeSource("ducking:os-monitor");
    if (mic) ctx.audio.duck.removeSource("ducking:mic");
    mic?.disable();
  });
</script>
