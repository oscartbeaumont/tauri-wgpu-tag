import "./App.css";
import { createSignal, Show } from "solid-js";

function App() {
  const [showTopLayer, setShowTopLayer] = createSignal(true);

  return (
    <main class="container">
      <h1>Tauri WGPU Tag Prototype</h1>
      <p>Native red slots are rendered by Rust + wgpu.</p>

      <section class="playground">
        <tauri-wgpu class="slot slot-a" />
        <tauri-wgpu class="slot slot-b" />

        <Show when={showTopLayer()}>
          <tauri-wgpu class="slot slot-c" />
        </Show>

        <div class="hud">
          <button type="button" onClick={() => setShowTopLayer((value) => !value)}>
            Toggle top layer
          </button>
          <p>Click-through check: this button should always be clickable.</p>
        </div>
      </section>
    </main>
  );
}

export default App;
