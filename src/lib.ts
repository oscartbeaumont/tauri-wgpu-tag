import { invoke } from "@tauri-apps/api/core";

type SlotSyncUpdate = {
  id: string;
  mounted: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
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

function scheduleFlush(): void {
  if (rafId !== null) {
    return;
  }

  rafId = window.requestAnimationFrame(() => {
    rafId = null;
    flushDirtyElements();
  });
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

function invalidateAll(): void {
  document
    .querySelectorAll<TauriWgpuElement>(TAG_NAME)
    .forEach((element) => DIRTY_ELEMENTS.add(element));
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

class TauriWgpuElement extends HTMLElement {
  static get observedAttributes(): string[] {
    return ["x", "y", "width", "height", "z-index"];
  }

  public slotId = "";

  private mounted = false;
  private slotSeq = 0;

  nextSlotSeq(): number {
    this.slotSeq += 1;
    return this.slotSeq;
  }

  attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void {
    if (oldValue === newValue) {
      return;
    }

    this.applyViewportGeometry();

    if (!this.mounted || !this.isConnected) {
      return;
    }

    if (name === "z-index") {
      invalidateAll();
    } else {
      DIRTY_ELEMENTS.add(this);
    }

    scheduleFlush();
  }

  private readNumberAttribute(name: string): number {
    const value = this.getAttribute(name);
    if (value === null) {
      return 0;
    }

    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : 0;
  }

  private applyViewportGeometry(): void {
    this.style.position = "fixed";
    this.style.left = `${this.readNumberAttribute("x")}px`;
    this.style.top = `${this.readNumberAttribute("y")}px`;
    this.style.width = `${Math.max(1, this.readNumberAttribute("width"))}px`;
    this.style.height = `${Math.max(1, this.readNumberAttribute("height"))}px`;
    this.style.zIndex = `${toNumber(this.getAttribute("z-index") ?? "0")}`;
    this.style.display = "block";
  }

  connectedCallback(): void {
    if (!this.slotId) {
      idCounter += 1;
      this.slotId = this.getAttribute("slot-id") ?? `tauri-wgpu-${idCounter}`;
      this.dataset.slotId = this.slotId;
    }

    this.applyViewportGeometry();
    this.mounted = true;
    invalidateAll();
    scheduleFlush();
  }

  disconnectedCallback(): void {
    this.mounted = false;
    DIRTY_ELEMENTS.delete(this);
    invalidateAll();
    scheduleFlush();

    const payload: SlotSyncUpdate = {
      id: this.slotId,
      mounted: false,
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      dpr: window.devicePixelRatio || 1,
      zIndex: 0,
      paintOrder: 0,
      slotSeq: this.nextSlotSeq(),
    };

    enqueueUpdate(payload);
  }

  flushSync(paintOrderMap: Map<string, number>): void {
    if (!this.mounted || !this.isConnected) {
      return;
    }

    const x = this.readNumberAttribute("x");
    const y = this.readNumberAttribute("y");
    const width = Math.max(1, this.readNumberAttribute("width"));
    const height = Math.max(1, this.readNumberAttribute("height"));
    const zIndex = toNumber(this.getAttribute("z-index") ?? "0");
    const paintOrder = paintOrderMap.get(this.slotId) ?? 0;

    const payload: SlotSyncUpdate = {
      id: this.slotId,
      mounted: true,
      x,
      y,
      width,
      height,
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
