use std::sync::OnceLock;

use super::testing::{FakeWaker, install_fakes};
use super::*;

/// Número de mensaje que solo usa este test, para que otros tests en paralelo no lo cuenten.
const MSG_MANY: u32 = crate::WM_APP + 0x7A01;

#[test]
fn waker_receives_posts_from_many_threads() {
    install_fakes();
    let before = FakeWaker::count(MSG_MANY);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for i in 0..100 {
                    Handle(1).post(MSG_MANY, i, 0);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(FakeWaker::count(MSG_MANY) - before, 800);
}

#[test]
fn install_fakes_is_idempotent() {
    install_fakes();
    install_fakes();
    let _ = waker();
    let _ = env();
    let _ = file_ops();
}

#[test]
fn posting_to_null_handle_is_harmless() {
    install_fakes();
    assert!(Handle(0).is_null());
    assert!(!Handle(1).is_null());
    Handle(0).post(crate::WM_APP + 0x7A02, 0, 0);
}

#[test]
#[should_panic(expected = "servicios del sistema no instalados")]
fn missing_install_panics_with_message() {
    let empty: OnceLock<Services> = OnceLock::new();
    let _ = get_or_panic(&empty);
}
