//! `settings`: a user's personal settings (personal instructions).

mod entities;
mod ports;
mod service;

pub use crate::domains::settings::entities::UserSettings;
pub use crate::domains::settings::ports::SettingsRepository;
pub use crate::domains::settings::service::SettingsService;
