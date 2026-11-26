//! `project`: reusable chat contexts (instructions, files, default model, reasoning, tools).

mod entities;
mod ports;
mod service;

pub use crate::domains::project::entities::{Project, ProjectParts};
pub use crate::domains::project::ports::{ProjectQuery, ProjectRepository};
pub use crate::domains::project::service::{NewProject, ProjectService, ProjectUpdate};
