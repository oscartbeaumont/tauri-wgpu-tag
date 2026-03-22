/// <reference types="vite/client" />

import "solid-js";

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      "tauri-wgpu": HTMLAttributes<HTMLElement>;
    }
  }
}
