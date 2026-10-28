use crate::time::error::Error;
use std::cell::RefCell;

thread_local! {
    static HOOK: RefCell<Option<Box<dyn FnOnce() -> Option<Error>>>> = RefCell::new(None);
}

pub(crate) struct Guard;

pub(crate) fn install(hook: impl FnOnce() -> Option<Error> + 'static) -> Guard {
    HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    Guard
}

pub(crate) fn run() -> Option<Error> {
    let hook = HOOK.with(|slot| slot.borrow_mut().take());
    hook.and_then(|hook| hook())
}

impl Drop for Guard {
    fn drop(&mut self) {
        HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}
