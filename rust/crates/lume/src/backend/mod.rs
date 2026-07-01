pub mod bundle;
pub mod capabilities;
pub mod descriptors;
pub mod diagnostics;
pub mod externals;

pub use bundle::{BackendBundle, BackendBundleResult, build_backend_bundle};
pub use descriptors::{
    BackendDescriptors, DescriptorField, DescriptorFunction, DescriptorGlobal, DescriptorModule,
    DescriptorOrigin, DescriptorType,
};
pub use externals::{ExternalDescriptors, ExternalSymbol, ExternalSymbolKind};
