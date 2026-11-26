use super::*;
use crate::lint::tree::violations;

#[test]
fn a_poisoned_lock_is_taken_back_instead_of_failing_or_dropped() {
    let source = "fn take(state: &Mutex<u8>, list: &RwLock<u8>) -> Result<(), Error> {
             let state = state.lock().unwrap();
             let list = list.read().expect(\"not poisoned\");
             let list = list.write()?;
             let state = state.lock().ok();
             let state = state.lock().unwrap_or_else(PoisonError::into_inner);
             let named = names.read(&mut buffer).unwrap();
             Ok(())
         }
         #[cfg(test)]
         mod tests {
             fn held(state: &Mutex<u8>) { state.lock().unwrap(); }
         }";

    assert_eq!(
        violations(&[("src/a.rs", source)], a_poisoned_lock_is_taken_back),
        [
            "src/a.rs:2: .lock().unwrap(..)",
            "src/a.rs:3: .read().expect(..)",
            "src/a.rs:4: .write()?",
            "src/a.rs:5: .lock().ok(..)",
        ]
    );
}
