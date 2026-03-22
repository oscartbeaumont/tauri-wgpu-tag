use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WgpuSlotSyncPayload {
    id: String,
    mounted: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpr: f64,
    z_index: i64,
    paint_order: i64,
    top: f64,
}

#[derive(Default)]
struct WgpuSlotState {
    slots: Mutex<HashMap<String, SlotEntry>>,
}

struct SlotEntry {
    #[cfg(target_os = "macos")]
    native_view_ptr: usize,
    #[cfg(target_os = "macos")]
    renderer: SlotRenderer,
    z_index: i64,
    paint_order: i64,
    top: f64,
}

#[cfg(target_os = "macos")]
struct SlotRenderer {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

#[cfg(target_os = "macos")]
impl SlotRenderer {
    async fn new(
        native_view_ptr: usize,
        width: f64,
        height: f64,
        dpr: f64,
    ) -> Result<Self, String> {
        use raw_window_handle::{
            AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
        };
        use std::{ffi::c_void, ptr::NonNull};

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let ns_view_ptr = NonNull::new(native_view_ptr as *mut c_void)
            .ok_or_else(|| "invalid native view pointer".to_string())?;

        let raw_window_handle = RawWindowHandle::AppKit(AppKitWindowHandle::new(ns_view_ptr));
        let raw_display_handle = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
        }
        .map_err(|e| format!("failed to create wgpu surface: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or_else(|| "failed to get adapter".to_string())?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tauri-wgpu-slot-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| format!("failed to create device: {e}"))?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| "surface has no supported formats".to_string())?;

        let present_mode = capabilities
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Fifo)
            .or_else(|| capabilities.present_modes.first().copied())
            .ok_or_else(|| "surface has no present mode".to_string())?;

        let alpha_mode = capabilities
            .alpha_modes
            .first()
            .copied()
            .ok_or_else(|| "surface has no alpha mode".to_string())?;

        let mut renderer = Self {
            _instance: instance,
            surface,
            device,
            queue,
            config: wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 1,
                height: 1,
                present_mode,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        };

        renderer.resize(width, height, dpr);
        renderer.render()?;
        Ok(renderer)
    }

    fn resize(&mut self, width: f64, height: f64, dpr: f64) {
        let px_width = (width * dpr).round().max(1.0) as u32;
        let px_height = (height * dpr).round().max(1.0) as u32;
        self.config.width = px_width;
        self.config.height = px_height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self) -> Result<(), String> {
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err("wgpu surface out of memory".to_string());
            }
            Err(wgpu::SurfaceError::Timeout) => {
                return Ok(());
            }
            Err(other) => {
                return Err(format!("failed to acquire surface texture: {other}"));
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tauri-wgpu-slot-encoder"),
            });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tauri-wgpu-slot-red-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn run_on_main_thread_sync<T: Send + 'static>(
    window: &WebviewWindow,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel::<Result<T, String>>();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| format!("failed to schedule main-thread task: {e}"))?;

    rx.recv()
        .map_err(|e| format!("failed to receive main-thread result: {e}"))?
}

#[cfg(target_os = "macos")]
unsafe fn create_or_get_overlay_view(content_view_ptr: usize, id: &str) -> Result<usize, String> {
    use cocoa::{
        base::{id as cocoa_id, nil, NO},
        foundation::{NSPoint, NSRect, NSSize, NSString},
    };
    use objc::{class, msg_send, runtime::Class, sel, sel_impl};

    unsafe fn passthrough_view_class() -> &'static Class {
        use cocoa::{base::id as cocoa_id, foundation::NSPoint};
        use objc::{
            declare::ClassDecl,
            runtime::{Class, Object, Sel},
            sel, sel_impl,
        };
        use std::sync::OnceLock;

        extern "C" fn hit_test(_: &Object, _: Sel, _: NSPoint) -> cocoa_id {
            cocoa::base::nil
        }

        static CLASS: OnceLock<&'static Class> = OnceLock::new();
        *CLASS.get_or_init(|| {
            let superclass = Class::get("NSView").expect("NSView class should exist");
            let mut decl = ClassDecl::new("TauriWgpuPassthroughView", superclass)
                .expect("failed to declare TauriWgpuPassthroughView class");

            unsafe {
                decl.add_method(
                    sel!(hitTest:),
                    hit_test as extern "C" fn(&Object, Sel, NSPoint) -> cocoa_id,
                );
            }

            decl.register()
        })
    }

    let content_view = content_view_ptr as cocoa_id;
    if content_view.is_null() {
        return Err("main content view is null".to_string());
    }

    let slot_id = format!("tauri-wgpu-slot-{id}");
    let identifier = NSString::alloc(nil).init_str(&slot_id);

    let existing: cocoa_id = msg_send![content_view, viewWithIdentifier: identifier];
    if !existing.is_null() {
        return Ok(existing as usize);
    }

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    let view_class = passthrough_view_class();
    let view: cocoa_id = msg_send![view_class, alloc];
    let view: cocoa_id = msg_send![view, initWithFrame: frame];
    let _: () = msg_send![view, setIdentifier: identifier];
    let _: () = msg_send![view, setWantsLayer: cocoa::base::YES];

    let layer: cocoa_id = msg_send![class!(CAMetalLayer), layer];
    let _: () = msg_send![view, setLayer: layer];
    let _: () = msg_send![view, setHidden: NO];

    let _: () = msg_send![content_view, addSubview: view];
    Ok(view as usize)
}

#[cfg(target_os = "macos")]
unsafe fn remove_overlay_view(content_view_ptr: usize, native_view_ptr: usize) {
    use cocoa::base::id as cocoa_id;
    use objc::{msg_send, sel, sel_impl};

    let _ = content_view_ptr;
    let native_view = native_view_ptr as cocoa_id;
    if !native_view.is_null() {
        let _: () = msg_send![native_view, removeFromSuperview];
    }
}

#[cfg(target_os = "macos")]
unsafe fn update_overlay_frame(
    content_view_ptr: usize,
    payload: &WgpuSlotSyncPayload,
) -> Result<(), String> {
    use cocoa::{
        appkit::NSView,
        base::id as cocoa_id,
        foundation::{NSPoint, NSRect, NSSize, NSString},
    };
    use objc::{msg_send, sel, sel_impl};

    let content_view = content_view_ptr as cocoa_id;
    if content_view.is_null() {
        return Err("main content view is null".to_string());
    }

    let slot_id = format!("tauri-wgpu-slot-{}", payload.id);
    let identifier = cocoa::foundation::NSString::alloc(cocoa::base::nil).init_str(&slot_id);
    let native_view: cocoa_id = msg_send![content_view, viewWithIdentifier: identifier];
    if native_view.is_null() {
        return Err("native slot view not found".to_string());
    }

    let content_bounds = NSView::bounds(content_view);
    let width = payload.width.max(1.0);
    let height = payload.height.max(1.0);
    let appkit_y = content_bounds.size.height - payload.y - height;

    let frame = NSRect::new(
        NSPoint::new(payload.x, appkit_y),
        NSSize::new(width, height),
    );
    let _: () = msg_send![native_view, setFrame: frame];
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe fn reorder_overlay_views(
    content_view_ptr: usize,
    native_view_ptrs: &[usize],
) -> Result<(), String> {
    use cocoa::base::id as cocoa_id;
    use objc::{msg_send, sel, sel_impl};

    let content_view = content_view_ptr as cocoa_id;
    if content_view.is_null() {
        return Err("main content view is null".to_string());
    }

    for ptr in native_view_ptrs {
        let view = *ptr as cocoa_id;
        if view.is_null() {
            continue;
        }
        let _: () = msg_send![view, removeFromSuperview];
        let _: () = msg_send![content_view, addSubview: view];
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn get_main_content_view_ptr(window: &WebviewWindow) -> Result<usize, String> {
    window
        .ns_view()
        .map(|ptr| ptr as usize)
        .map_err(|e| format!("failed to get main content view: {e}"))
}

#[cfg(target_os = "macos")]
fn sorted_native_views(map: &HashMap<String, SlotEntry>) -> Vec<usize> {
    let mut slots = map.values().collect::<Vec<_>>();
    slots.sort_by(|a, b| {
        a.z_index
            .cmp(&b.z_index)
            .then_with(|| a.paint_order.cmp(&b.paint_order))
            .then_with(|| {
                a.top
                    .partial_cmp(&b.top)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    slots.into_iter().map(|slot| slot.native_view_ptr).collect()
}

#[tauri::command]
async fn wgpu_slot_sync(
    app: AppHandle,
    state: State<'_, WgpuSlotState>,
    payload: WgpuSlotSyncPayload,
) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        let _ = state;
        let _ = payload;
        return Err("wgpu slots are currently implemented for macOS only".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "main window was not found".to_string())?;

        let content_view_ptr = get_main_content_view_ptr(&window)?;

        if !payload.mounted {
            let mut slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;

            if let Some(slot) = slots.remove(&payload.id) {
                let native_view_ptr = slot.native_view_ptr;
                run_on_main_thread_sync(&window, move || unsafe {
                    remove_overlay_view(content_view_ptr, native_view_ptr);
                    Ok(())
                })?;
            }

            return Ok(());
        }

        let needs_create = {
            let slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;
            !slots.contains_key(&payload.id)
        };

        if needs_create {
            let slot_id = payload.id.clone();
            let native_view_ptr = run_on_main_thread_sync(&window, move || unsafe {
                create_or_get_overlay_view(content_view_ptr, &slot_id)
            })?;

            let renderer =
                SlotRenderer::new(native_view_ptr, payload.width, payload.height, payload.dpr)
                    .await?;

            let mut slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;
            slots.insert(
                payload.id.clone(),
                SlotEntry {
                    native_view_ptr,
                    renderer,
                    z_index: payload.z_index,
                    paint_order: payload.paint_order,
                    top: payload.top,
                },
            );
        }

        {
            let mut slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;

            let slot = slots
                .get_mut(&payload.id)
                .ok_or_else(|| "slot should exist after create/update".to_string())?;

            slot.z_index = payload.z_index;
            slot.paint_order = payload.paint_order;
            slot.top = payload.top;
        }

        let reorder_ptrs = {
            let slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;
            sorted_native_views(&slots)
        };

        let payload_for_frame = payload.clone();
        run_on_main_thread_sync(&window, move || unsafe {
            update_overlay_frame(content_view_ptr, &payload_for_frame)?;
            reorder_overlay_views(content_view_ptr, &reorder_ptrs)?;
            Ok(())
        })?;

        {
            let mut slots = state
                .slots
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;
            let slot = slots
                .get_mut(&payload.id)
                .ok_or_else(|| "slot should exist for render".to_string())?;
            slot.renderer
                .resize(payload.width, payload.height, payload.dpr);
            slot.renderer.render()?;
        }

        Ok(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(WgpuSlotState::default())
        .invoke_handler(tauri::generate_handler![wgpu_slot_sync])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
