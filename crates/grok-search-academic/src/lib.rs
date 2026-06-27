mod institutional;
mod providers;
mod service;

pub use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
pub use service::AcademicService;
