use crate::apps::settings::SettingsApp;
use crate::apps::App;
use crate::buffer::Framebuffer;
use log::info;

pub type AppFactory = fn() -> Box<dyn App>;

pub struct AppManager {
    factories: Vec<AppFactory>,
    current_idx: usize,
    active_app: Box<dyn App>,
    settings_app: Option<SettingsApp>,
    show_settings: bool,
    was_connected: bool,
}

impl AppManager {
    pub fn new(factories: Vec<AppFactory>) -> Self {
        assert!(
            !factories.is_empty(),
            "Must provide at least one app factory"
        );
        let active_app = factories[0]();
        Self {
            factories,
            current_idx: 0,
            active_app,
            settings_app: Some(SettingsApp::new()),
            show_settings: true, // Show on startup to display "Connecting..."
            was_connected: false,
        }
    }

    pub fn next_app(&mut self) {
        self.current_idx = (self.current_idx + 1) % self.factories.len();
        self.active_app = self.factories[self.current_idx]();
        self.show_settings = false; // Auto-hide settings when cycling apps
        info!("Switched to app index {}", self.current_idx);
    }

    pub fn toggle_settings(&mut self) {
        self.show_settings = !self.show_settings;
        if self.show_settings && self.settings_app.is_none() {
            self.settings_app = Some(SettingsApp::new());
        } else if !self.show_settings {
            self.settings_app = None;
        }
    }

    pub fn update(
        &mut self,
        dt_ms: f32,
        is_connected: bool,
        ip: Option<String>,
    ) {
        // Auto-hide settings if we just connected and were previously disconnected
        if !self.was_connected && is_connected && self.show_settings {
            self.show_settings = false;
            self.settings_app = None;
        }
        self.was_connected = is_connected;

        if self.show_settings || !is_connected {
            if self.settings_app.is_none() {
                self.settings_app = Some(SettingsApp::new());
            }
            if let Some(app) = &mut self.settings_app {
                app.ip = ip;
                app.update(dt_ms);
            }
        } else {
            self.active_app.update(dt_ms);
        }
    }

    pub fn draw(&self, fb: &mut Framebuffer, is_connected: bool) {
        if self.show_settings || !is_connected {
            if let Some(app) = &self.settings_app {
                app.draw(fb);
            }
        } else {
            self.active_app.draw(fb);
        }
    }
}
