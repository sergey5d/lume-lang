/// Describes backend features that can vary by target.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackendCapabilities {
    pub supports_native_classes: bool,
    pub supports_native_records: bool,
    pub supports_checked_external_calls: bool,
}
