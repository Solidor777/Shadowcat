import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { AuraEmission, SoundEmission, VfxEmission } from "@shadowcat/core";
import EmissionEditor from "./EmissionEditor.svelte";

// Suppress listAssets fetch: the component's $effect calls listAssets, which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

describe("EmissionEditor", () => {
  it("toggles aura on with a default payload and off to null", async () => {
    const onAura = vi.fn();
    const { rerender } = render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura: null, sound: null, vfx: null, onAura, onSound: vi.fn(), onVfx: vi.fn() },
    });
    await fireEvent.click(screen.getByLabelText("actors.aura"));
    expect(onAura).toHaveBeenCalledWith({ color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true });

    const aura: AuraEmission = { color: "#111111", opacity: 0.5, radius: 3, enabled: true };
    await rerender({ aura, sound: null, vfx: null, onAura, onSound: vi.fn(), onVfx: vi.fn() });
    await fireEvent.click(screen.getByLabelText("actors.aura"));
    expect(onAura).toHaveBeenLastCalledWith(null);
  });

  it("toggles sound on with a default payload and off to null", async () => {
    const onSound = vi.fn();
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura: null, sound: null, vfx: null, onAura: vi.fn(), onSound, onVfx: vi.fn() },
    });
    await fireEvent.click(screen.getByLabelText("actors.sound"));
    expect(onSound).toHaveBeenCalledWith({ asset: "", radius: 5, volume: 0.8, loop: true, enabled: true });
  });

  it("toggles vfx on with a default payload and off to null", async () => {
    const onVfx = vi.fn();
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura: null, sound: null, vfx: null, onAura: vi.fn(), onSound: vi.fn(), onVfx },
    });
    await fireEvent.click(screen.getByLabelText("actors.vfx"));
    expect(onVfx).toHaveBeenCalledWith({ asset: "", anchor: "token", loop: true, enabled: true });
  });

  it("emits the replacement payload on a field edit", async () => {
    const onAura = vi.fn();
    const aura: AuraEmission = { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true };
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura, sound: null, vfx: null, onAura, onSound: vi.fn(), onVfx: vi.fn() },
    });
    await fireEvent.change(screen.getByLabelText("actors.auraRadius"), { target: { value: "5" } });
    expect(onAura).toHaveBeenCalledWith({ ...aura, radius: 5 });
  });

  it("edits a field on an already-toggled-on sound emission", async () => {
    const onSound = vi.fn();
    const sound: SoundEmission = { asset: "", radius: 5, volume: 0.8, loop: true, enabled: true };
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura: null, sound, vfx: null, onAura: vi.fn(), onSound, onVfx: vi.fn() },
    });
    await fireEvent.change(screen.getByLabelText("actors.emissionVolume"), { target: { value: "0.2" } });
    expect(onSound).toHaveBeenCalledWith({ ...sound, volume: 0.2 });
  });

  it("edits a field on an already-toggled-on vfx emission", async () => {
    const onVfx = vi.fn();
    const vfx: VfxEmission = { asset: "", anchor: "token", loop: true, enabled: true };
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura: null, sound: null, vfx, onAura: vi.fn(), onSound: vi.fn(), onVfx },
    });
    await fireEvent.click(screen.getByLabelText("actors.vfxLoop"));
    expect(onVfx).toHaveBeenCalledWith({ ...vfx, loop: false });
  });

  it("disables every toggle and field control when disabled is true", () => {
    const aura: AuraEmission = { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true };
    render(EmissionEditor, {
      context: setAppContextForTest({}),
      props: { aura, sound: null, vfx: null, onAura: vi.fn(), onSound: vi.fn(), onVfx: vi.fn(), disabled: true },
    });
    expect((screen.getByLabelText("actors.aura") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText("actors.sound") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText("actors.vfx") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText("actors.auraColor") as HTMLInputElement).disabled).toBe(true);
  });
});
