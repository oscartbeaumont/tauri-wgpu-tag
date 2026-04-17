import { invoke } from "@tauri-apps/api/core";

type SlotSyncUpdate = {
  id: string;
  mounted: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  logicalX: number;
  logicalY: number;
  logicalWidth: number;
  logicalHeight: number;
  viewportLogicalWidth: number;
  viewportLogicalHeight: number;
  dpr: number;
  zIndex: number;
  paintOrder: number;
  slotSeq: number;
};

type SlotSyncBatchPayload = {
  frameSeq: number;
  updates: SlotSyncUpdate[];
};

const TAG_NAME = "tauri-wgpu";
const DIRTY_ELEMENTS = new Set<TauriWgpuElement>();
const PENDING_UPDATES = new Map<string, SlotSyncUpdate>();

let rafId: number | null = null;
let syncInFlight = false;
let globalListenersInstalled = false;
let idCounter = 0;
let frameSeq = 0;

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

function getViewportLogicalSize(): { width: number; height: number } {
  const root = document.documentElement;
  return {
    width: root.clientWidth || window.innerWidth || 0,
    height: root.clientHeight || window.innerHeight || 0,
  };
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

async function sendSync(payload: SlotSyncBatchPayload): Promise<boolean> {
  try {
    await startupReady;

    await invoke("wgpu_slots_sync_batch", { payload });
    return true;
  } catch (error) {
    console.error("[tauri-wgpu] failed to sync batch", payload, error);
    return false;
  }
}

function mergePendingUpdate(update: SlotSyncUpdate): void {
  const previous = PENDING_UPDATES.get(update.id);
  if (!previous || update.slotSeq >= previous.slotSeq) {
    PENDING_UPDATES.set(update.id, update);
  }
}

function enqueueUpdate(update: SlotSyncUpdate): void {
  mergePendingUpdate(update);
  void flushPendingUpdatesToNative();
}

async function flushPendingUpdatesToNative(): Promise<void> {
  if (syncInFlight || PENDING_UPDATES.size === 0) {
    return;
  }

  if (document.visibilityState !== "visible") {
    return;
  }

  const updates = Array.from(PENDING_UPDATES.values());
  PENDING_UPDATES.clear();

  const payload: SlotSyncBatchPayload = {
    frameSeq: (frameSeq += 1),
    updates,
  };

  syncInFlight = true;

  try {
    const success = await sendSync(payload);
    if (!success) {
      for (const update of updates) {
        mergePendingUpdate(update);
      }
    }
  } finally {
    syncInFlight = false;
    if (PENDING_UPDATES.size > 0) {
      void flushPendingUpdatesToNative();
    }
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
    element.flushSync(paintOrderMap);
  }

  void flushPendingUpdatesToNative();
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
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      invalidateAll();
      void flushPendingUpdatesToNative();
    }
  });
  globalListenersInstalled = true;
}

class TauriWgpuElement extends HTMLElement {
  public slotId = "";

  private mounted = false;
  private resizeObserver: ResizeObserver | null = null;
  private mutationObserver: MutationObserver | null = null;
  private slotSeq = 0;

  nextSlotSeq(): number {
    this.slotSeq += 1;
    return this.slotSeq;
  }

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

    const payload: SlotSyncUpdate = {
      id: this.slotId,
      mounted: false,
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      logicalX: 0,
      logicalY: 0,
      logicalWidth: 0,
      logicalHeight: 0,
      viewportLogicalWidth: 0,
      viewportLogicalHeight: 0,
      dpr: window.devicePixelRatio || 1,
      zIndex: 0,
      paintOrder: 0,
      slotSeq: this.nextSlotSeq(),
    };

    enqueueUpdate(payload);
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

  flushSync(paintOrderMap: Map<string, number>): void {
    if (!this.mounted || !this.isConnected) {
      return;
    }

    const rect = this.getBoundingClientRect();
    const style = window.getComputedStyle(this);
    const zIndex = toNumber(style.zIndex);
    const paintOrder = paintOrderMap.get(this.slotId) ?? 0;
    const viewportLogicalSize = getViewportLogicalSize();

    const payload: SlotSyncUpdate = {
      id: this.slotId,
      mounted: true,
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
      logicalX: rect.left,
      logicalY: rect.top,
      logicalWidth: rect.width,
      logicalHeight: rect.height,
      viewportLogicalWidth: viewportLogicalSize.width,
      viewportLogicalHeight: viewportLogicalSize.height,
      dpr: window.devicePixelRatio || 1,
      zIndex,
      paintOrder,
      slotSeq: this.nextSlotSeq(),
    };

    enqueueUpdate(payload);
  }
}

if (!customElements.get(TAG_NAME)) {
  customElements.define(TAG_NAME, TauriWgpuElement);
}

export {};
