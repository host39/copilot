//! `provider`: connections to model providers and their models (capability defaults, locks,
//! dialects, and model sync rules).

mod capabilities;
mod entities;
mod model_service;
mod ports;
mod service;
mod values;

pub use crate::domains::provider::capabilities::{
    Capabilities, Capability, CapabilityOverrides, Lock, Locks, reasoning_off_with_tools,
};
pub use crate::domains::provider::entities::{
    KeyState, Model, ModelParts, Provider, ProviderParts,
};
pub use crate::domains::provider::model_service::{ModelService, ModelUpdate, ModelView};
pub use crate::domains::provider::ports::{
    DetectedModel, ModelQuery, ModelRepository, ProviderRepository, ProviderUsage, SyncReport,
};
pub use crate::domains::provider::service::{
    KeyChange, NewProvider, ProviderDetails, ProviderService, ProviderUpdate,
};
pub use crate::domains::provider::values::{ANTHROPIC_VERSION, Dialect, Preset, ProviderKind};
