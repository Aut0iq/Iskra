//! Minimal FFI wrapper around the installed C `gtk4-layer-shell` library
//! (pkg-config `gtk4-layer-shell-0`, 1.3.x headers). Only the calls the pet
//! needs are declared. Constants match `gtk-layer-shell.h`.

use std::ffi::c_void;

#[allow(non_camel_case_types)]
type gboolean = i32;

// GtkLayerShellLayer
pub const LAYER_TOP: i32 = 2;

// GtkLayerShellEdge
pub const EDGE_LEFT: i32 = 0;
pub const EDGE_TOP: i32 = 2;

// GtkLayerShellKeyboardMode
pub const KEYBOARD_MODE_NONE: i32 = 0;

extern "C" {
    fn gtk_layer_is_supported() -> gboolean;
    fn gtk_layer_is_layer_window(window: *mut c_void) -> gboolean;
    fn gtk_layer_init_for_window(window: *mut c_void);
    fn gtk_layer_set_namespace(window: *mut c_void, name_space: *const i8);
    fn gtk_layer_set_layer(window: *mut c_void, layer: i32);
    fn gtk_layer_set_anchor(window: *mut c_void, edge: i32, anchored: gboolean);
    fn gtk_layer_set_margin(window: *mut c_void, edge: i32, margin_size: i32);
    fn gtk_layer_set_exclusive_zone(window: *mut c_void, exclusive_zone: i32);
    fn gtk_layer_set_keyboard_mode(window: *mut c_void, mode: i32);
    fn gtk_layer_set_monitor(window: *mut c_void, monitor: *mut c_void);
}

/// True when the compositor supports the wlr layer-shell protocol.
pub fn is_supported() -> bool {
    unsafe { gtk_layer_is_supported() != 0 }
}

/// # Safety
/// `window` must be a valid `GtkWindow*`.
pub unsafe fn is_layer_window(window: *mut c_void) -> bool {
    gtk_layer_is_layer_window(window) != 0
}

/// # Safety
/// `window` must be a valid `GtkWindow*` obtained before it is realized for
/// `init_for_window`; the remaining setters are safe to call at any time on a
/// window that was already initialized.
pub unsafe fn init_for_window(window: *mut c_void) {
    gtk_layer_init_for_window(window);
}

/// # Safety
/// `window` must be a `GtkWindow*` previously passed to `init_for_window`.
pub unsafe fn configure_pet_window(window: *mut c_void, namespace: &std::ffi::CStr) {
    gtk_layer_set_namespace(window, namespace.as_ptr());
    gtk_layer_set_layer(window, LAYER_TOP);
    gtk_layer_set_anchor(window, EDGE_LEFT, 1);
    gtk_layer_set_anchor(window, EDGE_TOP, 1);
    gtk_layer_set_exclusive_zone(window, 0);
    gtk_layer_set_keyboard_mode(window, KEYBOARD_MODE_NONE);
}

/// # Safety
/// `window` must be an initialized layer window; `monitor` must be a valid
/// `GdkMonitor*` or null.
pub unsafe fn set_monitor(window: *mut c_void, monitor: *mut c_void) {
    gtk_layer_set_monitor(window, monitor);
}

/// # Safety
/// `window` must be an initialized layer window.
pub unsafe fn set_margins(window: *mut c_void, left: i32, top: i32) {
    gtk_layer_set_margin(window, EDGE_LEFT, left);
    gtk_layer_set_margin(window, EDGE_TOP, top);
}
