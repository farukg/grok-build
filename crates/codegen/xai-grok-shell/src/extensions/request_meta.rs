use agent_client_protocol as acp;

#[derive(serde::Deserialize)]
struct ExtensionTraceMeta {
    #[serde(rename = "_meta")]
    meta: Option<serde_json::Value>,
}

pub(crate) fn trace_meta(args: &acp::ExtRequest) -> Result<Option<serde_json::Value>, acp::Error> {
    if args.method.as_ref() == super::interject::QUEUE_INTERJECT_METHOD {
        super::interject::validate_queue_request_size(args.params.get())?;
    }
    Ok(
        serde_json::from_str::<ExtensionTraceMeta>(args.params.get())
            .ok()
            .and_then(|value| value.meta),
    )
}
