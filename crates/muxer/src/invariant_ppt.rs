// Adapted from muxide (MIT OR Apache-2.0)
//! Invariant PPT Testing Framework
//!
//! This module provides runtime invariant checking and contract test support
//! for Predictive Property-Based Testing (PPT).

use std::cell::RefCell;
use std::collections::HashSet;
use std::thread_local;

thread_local! {
    static INVARIANT_LOG: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Assert an invariant and log it for contract testing.
///
/// # Panics
/// Panics if the condition is false.
#[macro_export]
macro_rules! assert_invariant {
    ($condition:expr, $message:expr) => {
        $crate::invariant_ppt::__assert_invariant_impl($condition, $message, None)
    };
    ($condition:expr, $message:expr, $context:expr) => {
        $crate::invariant_ppt::__assert_invariant_impl($condition, $message, Some($context))
    };
}

/// Internal implementation - do not call directly
#[doc(hidden)]
pub fn __assert_invariant_impl(condition: bool, message: &str, context: Option<&str>) {
    #[cfg(debug_assertions)]
    INVARIANT_LOG.with(|log| {
        log.borrow_mut().insert(message.to_string());
    });

    if !condition {
        let ctx = context.unwrap_or("unknown");
        panic!("INVARIANT VIOLATION [{}]: {}", ctx, message);
    }
}

/// Check that specific invariants were verified during test execution.
pub fn contract_test(test_name: &str, required_invariants: &[&str]) {
    #[cfg(not(debug_assertions))]
    {
        let _ = (test_name, required_invariants);
        return;
    }

    #[cfg(debug_assertions)]
    {
        let log = INVARIANT_LOG.with(|log| log.borrow().clone());

        let mut missing: Vec<&str> = Vec::new();
        for invariant in required_invariants {
            if !log.contains(*invariant) {
                missing.push(invariant);
            }
        }

        if !missing.is_empty() {
            panic!(
                "CONTRACT FAILURE [{}]: The following invariants were not checked:\n  - {}",
                test_name,
                missing.join("\n  - ")
            );
        }
    }
}

/// Clear the invariant log (call between test runs if needed)
pub fn clear_invariant_log() {
    INVARIANT_LOG.with(|log| {
        log.borrow_mut().clear();
    });
}

/// Get a snapshot of currently logged invariants (for debugging)
pub fn get_logged_invariants() -> Vec<String> {
    INVARIANT_LOG.with(|log| log.borrow().iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invariant_log_records_and_clears() {
        clear_invariant_log();

        assert_invariant!(true, "poisoned invariant");

        #[cfg(debug_assertions)]
        {
            contract_test("poisoned", &["poisoned invariant"]);
            let logged = get_logged_invariants();
            assert!(logged.contains(&"poisoned invariant".to_string()));
        }
    }

    #[test]
    fn test_invariant_passes() {
        clear_invariant_log();
        assert_invariant!(true, "test invariant passes");

        #[cfg(debug_assertions)]
        {
            let logged = get_logged_invariants();
            assert!(logged.contains(&"test invariant passes".to_string()));
        }
    }

    #[test]
    #[should_panic(expected = "INVARIANT VIOLATION")]
    fn test_invariant_fails() {
        assert_invariant!(false, "this should fail", "test");
    }

    #[test]
    fn test_contract_passes() {
        clear_invariant_log();
        assert_invariant!(true, "contract required invariant");
        #[cfg(debug_assertions)]
        contract_test("test contract", &["contract required invariant"]);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "CONTRACT FAILURE")]
    fn test_contract_fails_missing() {
        clear_invariant_log();
        contract_test("test missing", &["this invariant was never checked"]);
    }
}
