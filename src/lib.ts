import { invoke } from "@tauri-apps/api/core";

type SlotSyncPayload = {
  id: string;
  mounted: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  dpr: number;
  zIndex: number;
  paintOrder: number;
  top: number;
};

const TAG_NAME = "tauri-wgpu";
const DIRTY_ELEMENTS = new Set<TauriWgpuElement>();

let rafId: number | null = null;
let globalListenersInstalled = false;
let idCounter = 0;

const startupReady = new Promise<void>((resolve) => {
  const finish = () => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  };

  if (document.readyState === "complete") {
    finish();
    return;
  }

  window.addEventListener("load", finish, { once: true });
});

function toNumber(value: string): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function scheduleFlush(): void {
  if (rafId !== null) {
    return;
  }

  rafId = window.requestAnimationFrame(() => {
    rafId = null;
    flushDirtyElements();
  });
}

function markDirty(element: TauriWgpuElement): void {
  DIRTY_ELEMENTS.add(element);
  scheduleFlush();
}

function getPaintOrderMap(): Map<string, number> {
  const map = new Map<string, number>();
  const elements = Array.from(document.querySelectorAll<TauriWgpuElement>(TAG_NAME));

  elements.forEach((element, index) => {
    if (element.slotId) {
      map.set(element.slotId, index);
    }
  });

  return map;
}

async function sendSync(payload: SlotSyncPayload): Promise<void> {
  try {
    await startupReady;

    if (document.visibilityState !== "visible") {
      return;
    }

    await invoke("wgpu_slot_sync", { payload });
  } catch (error) {
    console.error("[tauri-wgpu] failed to sync slot", payload.id, error);
  }
}

function flushDirtyElements(): void {
  if (DIRTY_ELEMENTS.size === 0) {
    return;
  }

  const paintOrderMap = getPaintOrderMap();
  const pending = Array.from(DIRTY_ELEMENTS);
  DIRTY_ELEMENTS.clear();

  for (const element of pending) {
    void element.flushSync(paintOrderMap);
  }
}

function installGlobalListeners(): void {
  if (globalListenersInstalled) {
    return;
  }

  const invalidateAll = (): void => {
    document
      .querySelectorAll<TauriWgpuElement>(TAG_NAME)
      .forEach((element) => markDirty(element));
  };

  window.addEventListener("scroll", invalidateAll, { passive: true });
  window.addEventListener("resize", invalidateAll, { passive: true });
  globalListenersInstalled = true;
}

class TauriWgpuElement extends HTMLElement {
  public slotId = "";

  private mounted = false;
  private resizeObserver: ResizeObserver | null = null;
  private mutationObserver: MutationObserver | null = null;

  connectedCallback(): void {
    if (!this.slotId) {
      idCounter += 1;
      this.slotId = this.getAttribute("slot-id") ?? `tauri-wgpu-${idCounter}`;
      this.dataset.slotId = this.slotId;
    }

    if (!this.style.display) {
      this.style.display = "block";
    }

    this.mounted = true;
    installGlobalListeners();
    this.setupObservers();
    markDirty(this);
    window.setTimeout(() => markDirty(this), 250);
    window.setTimeout(() => markDirty(this), 1000);
  }

  disconnectedCallback(): void {
    this.mounted = false;
    DIRTY_ELEMENTS.delete(this);
    this.cleanupObservers();

    const payload: SlotSyncPayload = {
      id: this.slotId,
      mounted: false,
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      dpr: window.devicePixelRatio || 1,
      zIndex: 0,
      paintOrder: 0,
      top: 0,
    };

    void sendSync(payload);
  }

  setupObservers(): void {
    if (!this.resizeObserver) {
      this.resizeObserver = new ResizeObserver(() => markDirty(this));
      this.resizeObserver.observe(this);
    }

    if (!this.mutationObserver) {
      this.mutationObserver = new MutationObserver(() => markDirty(this));
      this.mutationObserver.observe(this, {
        attributes: true,
        attributeFilter: ["style", "class"],
      });
    }
  }

  cleanupObservers(): void {
    this.resizeObserver?.disconnect();
    this.resizeObserver = null;

    this.mutationObserver?.disconnect();
    this.mutationObserver = null;
  }

  async flushSync(paintOrderMap: Map<string, number>): Promise<void> {
    if (!this.mounted || !this.isConnected) {
      return;
    }

    const rect = this.getBoundingClientRect();
    const style = window.getComputedStyle(this);
    const zIndex = toNumber(style.zIndex);
    const paintOrder = paintOrderMap.get(this.slotId) ?? 0;

    const payload: SlotSyncPayload = {
      id: this.slotId,
      mounted: true,
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
      dpr: window.devicePixelRatio || 1,
      zIndex,
      paintOrder,
      top: rect.top,
    };

    await sendSync(payload);
  }
}

if (!customElements.get(TAG_NAME)) {
  customElements.define(TAG_NAME, TauriWgpuElement);
}

export {};
