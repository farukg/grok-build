use super::*;

#[derive(Clone, Copy)]
enum ScopeExit { EarlyError, Unwind, Consumed, Unconsumed }

#[tokio::test]
async fn atomic_fault_scope_restores_same_worker_on_every_exit() {
    tokio::task::spawn_blocking(|| {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unrelated.json");
        for exit in [ScopeExit::EarlyError, ScopeExit::Unwind, ScopeExit::Consumed, ScopeExit::Unconsumed] {
            let _outer = AtomicWriteFaultScope::enter();
            fail_next_atomic_write(AtomicWriteFault::ParentSync);
            let result = std::panic::catch_unwind(|| -> io::Result<()> {
                let _scope = AtomicWriteFaultScope::enter();
                fail_next_atomic_write(AtomicWriteFault::Sync);
                match exit {
                    ScopeExit::EarlyError => return Err(io::Error::other("pre-append error")),
                    ScopeExit::Unwind => panic!("pre-append unwind"),
                    ScopeExit::Consumed => {
                        let error = write_bytes_atomic(&path, b"scoped bytes").unwrap_err();
                        assert_eq!(error.get_ref().unwrap().downcast_ref::<AtomicWriteFailure>().unwrap().fault, AtomicWriteFault::Sync);
                    }
                    ScopeExit::Unconsumed => {}
                }
                Ok(())
            });
            match exit {
                ScopeExit::EarlyError => assert!(result.unwrap().is_err()),
                ScopeExit::Unwind => assert!(result.is_err()),
                ScopeExit::Consumed | ScopeExit::Unconsumed => result.unwrap().unwrap(),
            }
            assert_eq!(ATOMIC_WRITE_FAULT.take(), Some(AtomicWriteFault::ParentSync), "restore the prior scope, not an unconditional empty flag");
            write_bytes_atomic(&path, b"unrelated next write").unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"unrelated next write");
            assert_eq!(ATOMIC_WRITE_FAULT.get(), None);
        }
    }).await.unwrap();
}
