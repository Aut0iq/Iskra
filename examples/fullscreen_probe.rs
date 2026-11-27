//! Disposable fullscreen probe: a normal GTK window that goes fullscreen
//! for ~2.5s then restores and exits. Used to verify the pet's hide/show
//! behavior without touching any user application.

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
use gtk4::glib;
#[cfg(unix)]
use gtk4::prelude::*;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn main() {
    let app = gtk4::Application::new(
        Some("dev.zeron.pet.fullscreen-probe"),
        gtk4::gio::ApplicationFlags::empty(),
    );
    app.connect_activate(|app| {
        let win = gtk4::ApplicationWindow::new(app);
        win.set_title(Some("zeron-pet-fullscreen-probe"));
        win.set_default_size(220, 120);
        let label = gtk4::Label::new(Some("probe"));
        win.set_child(Some(&label));
        win.present();

        let w = win.clone();
        glib::timeout_add_local_once(Duration::from_millis(700), move || {
            w.fullscreen();
        });
        let w = win.clone();
        glib::timeout_add_local_once(Duration::from_millis(3200), move || {
            w.unfullscreen();
        });
        let app2 = app.clone();
        glib::timeout_add_local_once(Duration::from_millis(3800), move || {
            app2.quit();
        });
    });
    app.run();
}
