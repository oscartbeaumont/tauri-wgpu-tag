/// <reference types="vite/client" />

import "solid-js";
import type { HTMLAttributes } from "solid-js";

type TauriWgpuAttributes = HTMLAttributes<HTMLElement> & {
  x?: string | number;
  y?: string | number;
  width?: string | number;
  height?: string | number;
  "z-index"?: string | number;
  "slot-id"?: string;
};

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      "tauri-wgpu": TauriWgpuAttributes;
    }
  }
}
