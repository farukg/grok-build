/// Whether the previous leader run stopped within the resume window.
///
/// Decided exactly once at leader startup (see the shell's `leader::restart_policy`)
/// and carried as a fact into session recovery and scheduler restore, so consumers
/// match the classification instead of recomputing wall-clock age themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestartStopAge {
    /// Last stop within the grace window, or a first run with no prior leader
    /// state: previously running work keeps running, quiescent stays quiescent.
    #[default]
    Fresh,
    /// Last stop older than the grace window (or an unreadable heartbeat):
    /// previously running obligations are cancelled once at startup.
    Stale,
}
