import "./App.css";
import { createSignal, Show } from "solid-js";

function App() {
  const [showTopLayer, setShowTopLayer] = createSignal(true);

  return (
    <main class="container">
      <h1>Tauri WGPU Tag Prototype</h1>
      <p>Native red slots are rendered by Rust + wgpu.</p>

      <section class="playground">
        <tauri-wgpu class="slot" x="64" y="160" width="260" height="170" z-index="1" />
        <tauri-wgpu class="slot" x="170" y="228" width="280" height="170" z-index="3" />

        <Show when={showTopLayer()}>
          <tauri-wgpu class="slot" x="310" y="148" width="230" height="138" z-index="5" />
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
