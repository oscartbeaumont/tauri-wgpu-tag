# tauri-wgpu-tag

A prototype of replicating `<electrobun-wgpu>` w/ Tauri

## Usage

```bash
pnpm i
pnpm tauri dev
```

## TODO

 - [ ] Document this works best for fixed-size elements? Can we force it? Maybe have a custom way of resizing which can be coordinated with Rust?
 - [ ] Allow moving the GPU surface around the screen in an animation like I did for Spacedrive with the SolidJS interop. Maybe we need a commit phase?
 - [ ] Allow putting GPU surface behind `body`/WebView content. For something like Cap it could make the resize flow look nicer.

 - [ ] Can this be more generic than wgpu, just native slots???
 - [ ] Multiple native surfaces visible at once
 - [ ] Mouse click events should pass-through
 - [ ] Windows support
 - [ ] macOS support
 - [ ] Convert into generic Tauri plugin

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
