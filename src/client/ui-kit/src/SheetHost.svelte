<script lang="ts">
  // Generic wrapper the sheets controller mounts around EVERY module sheet: renders the
  // host-owned template chrome above the picked sheet body, so any doc_type gets template
  // controls without opting in. `inner` is the picked sheet component; props are forwarded.
  import type { Component } from "svelte";
  import TemplateControls from "./TemplateControls.svelte";

  let { docId, systemPrefix, close, inner }: {
    /** The write-target document id, forwarded to `TemplateControls` and `inner`. */
    docId: string;
    /** The write-target's system-body prefix, forwarded to `inner`. */
    systemPrefix: string;
    /** Closes this sheet's panel; forwarded to `inner` for its own Close control. */
    close: () => void;
    /** The doc_type-picked sheet component this host wraps with template chrome. */
    inner: Component<Record<string, unknown>>;
  } = $props();

  const Inner = $derived(inner);
</script>

<div class="sheet-host">
  <TemplateControls {docId} />
  <div class="sheet-body">
    <Inner {docId} {systemPrefix} {close} />
  </div>
</div>

<style lang="scss">
  .sheet-host { display: flex; flex-direction: column; height: 100%; }
  .sheet-body { flex: 1; min-height: 0; overflow: auto; }
</style>
