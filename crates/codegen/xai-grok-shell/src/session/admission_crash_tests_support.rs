#[derive(Clone, Copy)]
pub(crate) enum AdmissionCrashBoundary { ReceiverAdmitted, InputProjected, ReceiverTerminal, ProducerPrepared, ProducerPublished }

pub(crate) fn crash_at(boundary: AdmissionCrashBoundary) {
    let name = match boundary {
        AdmissionCrashBoundary::ReceiverAdmitted => "receiver_admitted",
        AdmissionCrashBoundary::InputProjected => "input_projected",
        AdmissionCrashBoundary::ReceiverTerminal => "receiver_terminal",
        AdmissionCrashBoundary::ProducerPrepared => "producer_prepared",
        AdmissionCrashBoundary::ProducerPublished => "producer_published",
    };
    if std::env::var("GROK_ADMISSION_TEST_CRASH").ok().as_deref() == Some(name) { std::process::exit(86); }
}
