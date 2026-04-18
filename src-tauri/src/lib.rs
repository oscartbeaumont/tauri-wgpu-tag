use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
};
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WgpuSlotSyncPayload {
    id: String,
    mounted: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    logical_x: f64,
    logical_y: f64,
    logical_width: f64,
    logical_height: f64,
    viewport_logical_width: f64,
    viewport_logical_height: f64,
    dpr: f64,
    z_index: i64,
    paint_order: i64,
    slot_seq: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WgpuSlotsSyncBatchPayload {
    frame_seq: u64,
    updates: Vec<WgpuSlotSyncPayload>,
}

#[derive(Default)]
struct WgpuSlotState {
    inner: Arc<Mutex<WgpuSlotInnerState>>,
}

#[derive(Default)]
struct WgpuSlotInnerState {
    slots: HashMap<String, SlotEntry>,
    native_order: Vec<String>,
    last_slot_seq: HashMap<String, u64>,
}

struct SlotEntry {
    #[cfg(target_os = "macos")]
    native_view_ptr: usize,
    #[cfg(target_os = "macos")]
    renderer: Option<SlotRenderer>,
    z_index: i64,
    paint_order: i64,
    last_pixel_width: u32,
    last_pixel_height: u32,
    last_dpr: f64,
    logical_x: f64,
    logical_y: f64,
    logical_width: f64,
    logical_height: f64,
    viewport_logical_width: f64,
    viewport_logical_height: f64,
    last_native_content_width: f64,
    last_native_content_height: f64,
    last_applied_x: f64,
    last_applied_y: f64,
    last_applied_width: f64,
    last_applied_height: f64,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct EstimatedSlotFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
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
    fn new(native_view_ptr: usize, width: f64, height: f64, dpr: f64) -> Result<Self, String> {
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

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| "failed to get adapter".to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("tauri-wgpu-slot-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
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
unsafe fn create_overlay_view(content_view_ptr: usize) -> Result<usize, String> {
    use cocoa::{
        base::{id as cocoa_id, NO},
        foundation::{NSPoint, NSRect, NSSize},
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

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    let view_class = passthrough_view_class();
    let view: cocoa_id = msg_send![view_class, alloc];
    let view: cocoa_id = msg_send![view, initWithFrame: frame];
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
    native_view_ptr: usize,
    content_height: f64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpr: f64,
) -> Result<(), String> {
    use cocoa::{
        base::id as cocoa_id,
        foundation::{NSPoint, NSRect, NSSize},
    };
    use objc::{msg_send, sel, sel_impl};

    let native_view = native_view_ptr as cocoa_id;
    if native_view.is_null() {
        return Err("native slot view not found".to_string());
    }

    let width = width.max(1.0);
    let height = height.max(1.0);
    let appkit_y = content_height - y - height;

    let frame = NSRect::new(NSPoint::new(x, appkit_y), NSSize::new(width, height));
    let _: () = msg_send![native_view, setFrame: frame];

    let layer: cocoa_id = msg_send![native_view, layer];
    if !layer.is_null() {
        let _: () = msg_send![layer, setContentsScale: dpr];
    }

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
unsafe fn get_content_view_size(content_view_ptr: usize) -> Result<(f64, f64), String> {
    use cocoa::{appkit::NSView, base::id as cocoa_id};

    let content_view = content_view_ptr as cocoa_id;
    if content_view.is_null() {
        return Err("main content view is null".to_string());
    }

    let bounds = NSView::bounds(content_view);
    Ok((bounds.size.width, bounds.size.height))
}

#[cfg(target_os = "macos")]
fn sorted_slot_ids(map: &HashMap<String, SlotEntry>) -> Vec<String> {
    let mut slots = map.iter().collect::<Vec<_>>();
    slots.sort_by(|(id_a, a), (id_b, b)| {
        a.z_index
            .cmp(&b.z_index)
            .then_with(|| a.paint_order.cmp(&b.paint_order))
            .then_with(|| id_a.cmp(id_b))
    });

    slots.into_iter().map(|(id, _)| id.clone()).collect()
}

#[cfg(target_os = "macos")]
fn pixel_size(width: f64, height: f64, dpr: f64) -> (u32, u32) {
    let px_width = (width * dpr).round().max(1.0) as u32;
    let px_height = (height * dpr).round().max(1.0) as u32;
    (px_width, px_height)
}

#[cfg(target_os = "macos")]
fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(target_os = "macos")]
fn estimate_axis_position(
    leading: f64,
    trailing: f64,
    size: f64,
    old_viewport_size: f64,
    new_viewport_size: f64,
) -> f64 {
    let old_available = (old_viewport_size - size).max(0.0);
    if old_available <= f64::EPSILON {
        return 0.0;
    }

    let normalized_position = clamp01(leading / old_available);
    let center_band = 0.18;
    if (normalized_position - 0.5).abs() <= center_band {
        return (leading + (new_viewport_size - old_viewport_size) / 2.0).max(0.0);
    }

    if normalized_position < 0.5 {
        leading.max(0.0)
    } else {
        (new_viewport_size - trailing - size).max(0.0)
    }
}

#[cfg(target_os = "macos")]
fn estimate_slot_frame(
    slot: &SlotEntry,
    new_viewport_width: f64,
    new_viewport_height: f64,
) -> Option<EstimatedSlotFrame> {
    if slot.last_native_content_width <= 0.0 || slot.last_native_content_height <= 0.0 {
        return None;
    }

    let width = slot.last_applied_width.max(1.0);
    let height = slot.last_applied_height.max(1.0);
    let width_changed = (new_viewport_width - slot.last_native_content_width).abs() > f64::EPSILON;
    let height_changed =
        (new_viewport_height - slot.last_native_content_height).abs() > f64::EPSILON;
    let right = (slot.last_native_content_width - slot.last_applied_x - width).max(0.0);
    let bottom = (slot.last_native_content_height - slot.last_applied_y - height).max(0.0);

    let x = if width_changed {
        estimate_axis_position(
            slot.last_applied_x,
            right,
            width,
            slot.last_native_content_width,
            new_viewport_width,
        )
    } else {
        slot.last_applied_x
    };
    let y = if height_changed {
        estimate_axis_position(
            slot.last_applied_y,
            bottom,
            height,
            slot.last_native_content_height,
            new_viewport_height,
        )
    } else {
        slot.last_applied_y
    };

    Some(EstimatedSlotFrame {
        x,
        y,
        width,
        height,
    })
}

#[cfg(target_os = "macos")]
fn update_cached_applied_frame(
    slot: &mut SlotEntry,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    content_width: f64,
    content_height: f64,
) {
    slot.last_native_content_width = content_width;
    slot.last_native_content_height = content_height;
    slot.last_applied_x = x;
    slot.last_applied_y = y;
    slot.last_applied_width = width;
    slot.last_applied_height = height;
}

#[cfg(target_os = "macos")]
fn sync_estimated_overlay_frames(
    window: &WebviewWindow,
    inner: Arc<Mutex<WgpuSlotInnerState>>,
) -> Result<(), String> {
    let content_view_ptr = get_main_content_view_ptr(window)?;
    let dpr = window
        .scale_factor()
        .map_err(|e| format!("failed to get window scale factor: {e}"))?;

    run_on_main_thread_sync(window, move || {
        let mut inner = inner
            .lock()
            .map_err(|_| "slot state lock poisoned".to_string())?;
        let (content_width, content_height) = unsafe { get_content_view_size(content_view_ptr)? };
        let _transaction = unsafe { NoAnimationTransaction::begin() };

        for slot in inner.slots.values_mut() {
            let Some(frame) = estimate_slot_frame(slot, content_width, content_height) else {
                continue;
            };

            unsafe {
                update_overlay_frame(
                    slot.native_view_ptr,
                    content_height,
                    frame.x,
                    frame.y,
                    frame.width,
                    frame.height,
                    dpr,
                )?;
            }

            if let Some(renderer) = slot.renderer.as_mut() {
                let (px_width, px_height) = pixel_size(frame.width, frame.height, dpr);
                if slot.last_pixel_width != px_width
                    || slot.last_pixel_height != px_height
                    || (slot.last_dpr - dpr).abs() > f64::EPSILON
                {
                    renderer.resize(frame.width, frame.height, dpr);
                    slot.last_pixel_width = px_width;
                    slot.last_pixel_height = px_height;
                    slot.last_dpr = dpr;
                }
                renderer.render()?;
            }

            update_cached_applied_frame(
                slot,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                content_width,
                content_height,
            );
        }

        Ok(())
    })
}

#[cfg(target_os = "macos")]
struct NoAnimationTransaction;

#[cfg(target_os = "macos")]
impl NoAnimationTransaction {
    unsafe fn begin() -> Self {
        use cocoa::base::YES;
        use objc::{class, msg_send, sel, sel_impl};

        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
        Self
    }
}

#[cfg(target_os = "macos")]
impl Drop for NoAnimationTransaction {
    fn drop(&mut self) {
        use objc::{class, msg_send, sel, sel_impl};

        unsafe {
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }
}

#[tauri::command]
fn wgpu_slots_sync_batch(
    app: AppHandle,
    state: State<'_, WgpuSlotState>,
    payload: WgpuSlotsSyncBatchPayload,
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
        let _ = payload.frame_seq;

        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "main window was not found".to_string())?;

        let content_view_ptr = get_main_content_view_ptr(&window)?;
        let inner = state.inner.clone();

        run_on_main_thread_sync(&window, move || {
            let mut inner = inner
                .lock()
                .map_err(|_| "slot state lock poisoned".to_string())?;
            let (content_width, content_height) =
                unsafe { get_content_view_size(content_view_ptr)? };
            let _transaction = unsafe { NoAnimationTransaction::begin() };

            for update in payload.updates {
                let last_seq = inner.last_slot_seq.get(&update.id).copied().unwrap_or(0);
                if update.slot_seq <= last_seq {
                    continue;
                }
                inner
                    .last_slot_seq
                    .insert(update.id.clone(), update.slot_seq);

                if !update.mounted {
                    if let Some(slot) = inner.slots.remove(&update.id) {
                        unsafe {
                            remove_overlay_view(content_view_ptr, slot.native_view_ptr);
                        }
                    }
                    continue;
                }

                let native_view_ptr = match inner.slots.get(&update.id) {
                    Some(slot) => slot.native_view_ptr,
                    None => {
                        let native_view_ptr = unsafe { create_overlay_view(content_view_ptr)? };
                        inner.slots.insert(
                            update.id.clone(),
                            SlotEntry {
                                native_view_ptr,
                                renderer: None,
                                z_index: update.z_index,
                                paint_order: update.paint_order,
                                last_pixel_width: 0,
                                last_pixel_height: 0,
                                last_dpr: 0.0,
                                logical_x: update.logical_x,
                                logical_y: update.logical_y,
                                logical_width: update.logical_width,
                                logical_height: update.logical_height,
                                viewport_logical_width: update.viewport_logical_width,
                                viewport_logical_height: update.viewport_logical_height,
                                last_native_content_width: content_width,
                                last_native_content_height: content_height,
                                last_applied_x: update.x,
                                last_applied_y: update.y,
                                last_applied_width: update.width,
                                last_applied_height: update.height,
                            },
                        );
                        native_view_ptr
                    }
                };

                {
                    let slot = inner
                        .slots
                        .get_mut(&update.id)
                        .ok_or_else(|| "slot should exist after create/update".to_string())?;
                    slot.z_index = update.z_index;
                    slot.paint_order = update.paint_order;
                    slot.logical_x = update.logical_x;
                    slot.logical_y = update.logical_y;
                    slot.logical_width = update.logical_width;
                    slot.logical_height = update.logical_height;
                    slot.viewport_logical_width = update.viewport_logical_width;
                    slot.viewport_logical_height = update.viewport_logical_height;
                    update_cached_applied_frame(
                        slot,
                        update.x,
                        update.y,
                        update.width,
                        update.height,
                        content_width,
                        content_height,
                    );
                }

                unsafe {
                    update_overlay_frame(
                        native_view_ptr,
                        content_height,
                        update.x,
                        update.y,
                        update.width,
                        update.height,
                        update.dpr,
                    )?;
                }

                {
                    let slot = inner
                        .slots
                        .get_mut(&update.id)
                        .ok_or_else(|| "slot should exist for render".to_string())?;
                    if slot.renderer.is_none() {
                        slot.renderer = Some(SlotRenderer::new(
                            slot.native_view_ptr,
                            update.width,
                            update.height,
                            update.dpr,
                        )?);
                        let (px_width, px_height) =
                            pixel_size(update.width, update.height, update.dpr);
                        slot.last_pixel_width = px_width;
                        slot.last_pixel_height = px_height;
                        slot.last_dpr = update.dpr;
                    }

                    if let Some(renderer) = slot.renderer.as_mut() {
                        let (px_width, px_height) =
                            pixel_size(update.width, update.height, update.dpr);
                        if slot.last_pixel_width != px_width
                            || slot.last_pixel_height != px_height
                            || (slot.last_dpr - update.dpr).abs() > f64::EPSILON
                        {
                            renderer.resize(update.width, update.height, update.dpr);
                            slot.last_pixel_width = px_width;
                            slot.last_pixel_height = px_height;
                            slot.last_dpr = update.dpr;
                        }
                        renderer.render()?;
                    }
                }
            }

            let target_order = sorted_slot_ids(&inner.slots);
            if target_order != inner.native_order {
                let reorder_ptrs = target_order
                    .iter()
                    .filter_map(|slot_id| inner.slots.get(slot_id).map(|slot| slot.native_view_ptr))
                    .collect::<Vec<_>>();
                unsafe {
                    reorder_overlay_views(content_view_ptr, &reorder_ptrs)?;
                }
                inner.native_order = target_order;
            }

            Ok(())
        })
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(WgpuSlotState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let window = app
                    .get_webview_window("main")
                    .ok_or_else(|| "main window was not found".to_string())?;
                let inner = app.state::<WgpuSlotState>().inner.clone();
                let event_window = window.clone();

                window.on_window_event(move |event| {
                    if !matches!(
                        event,
                        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        return;
                    }

                    if let Err(error) = sync_estimated_overlay_frames(&event_window, inner.clone())
                    {
                        eprintln!("[tauri-wgpu] failed to estimate resize frames: {error}");
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![wgpu_slots_sync_batch])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
