use std::collections::HashMap;
use apollo_compiler::Name;
use crate::error::FederationError;

/// Error codes matching TypeScript implementation
pub const FIELD_TYPE_MISMATCH: &str = "FIELD_TYPE_MISMATCH";
pub const ARGUMENT_TYPE_MISMATCH: &str = "ARGUMENT_TYPE_MISMATCH";

/// Hint codes matching TypeScript implementation  
pub const INCONSISTENT_BUT_COMPATIBLE_FIELD_TYPE: &str = "INCONSISTENT_BUT_COMPATIBLE_FIELD_TYPE";
pub const INCONSISTENT_BUT_COMPATIBLE_ARGUMENT_TYPE: &str = "INCONSISTENT_BUT_COMPATIBLE_ARGUMENT_TYPE";

/// Structured error reporting for merge operations
/// Provides clean integration with existing error system while adding
/// TypeScript-compatible error codes and detailed messages
pub struct MismatchReporter {
    errors: Vec<String>,
    hints: Vec<String>,
}

impl MismatchReporter {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            hints: Vec::new(),
        }
    }

    /// Report a mismatch error with structured code and message
    /// Matches TypeScript reportMismatchError behavior
    pub fn report_mismatch_error(
        &mut self,
        error_code: &str,
        message: String,
        _element_coordinate: &str, // TODO: Add coordinate tracking
        _sources: &HashMap<String, String>, // TODO: Add source tracking  
        _field_formatter: impl Fn(&str) -> String, // TODO: Add field formatting
    ) {
        let formatted_error = format!("[{}] {}", error_code, message);
        self.errors.push(formatted_error);
    }

    /// Report a mismatch hint for compatible but inconsistent types
    /// Matches TypeScript reportMismatchHint behavior
    pub fn report_mismatch_hint(
        &mut self,
        hint_code: &str,
        message: String,
        _element_coordinate: &str, // TODO: Add coordinate tracking
        _sources: &HashMap<String, String>, // TODO: Add source tracking
    ) {
        let formatted_hint = format!("[{}] {}", hint_code, message);
        self.hints.push(formatted_hint);
    }

    /// Get accumulated errors
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Get accumulated hints  
    pub fn hints(&self) -> &[String] {
        &self.hints
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Check if there are any hints
    pub fn has_hints(&self) -> bool {
        !self.hints.is_empty()
    }

    /// Clear all errors and hints
    pub fn clear(&mut self) {
        self.errors.clear();
        self.hints.clear();
    }
}

impl Default for MismatchReporter {
    fn default() -> Self {
        Self::new()
    }
}