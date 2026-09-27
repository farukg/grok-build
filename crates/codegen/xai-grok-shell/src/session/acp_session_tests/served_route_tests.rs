use xai_grok_sampling_types::{FallbackCount, RouteModel, RouteProvider, ServedRoute};

use super::support::build_actor;

fn route(model: &str) -> ServedRoute {
    ServedRoute {
        provider: RouteProvider::parse("openai").unwrap(),
        model: RouteModel::parse(model).unwrap(),
        candidate: None,
        identity: None,
        profile: None,
        fallback_count: FallbackCount(1),
    }
}

fn served_routes_on_wire(
    gateway_rx: &mut tokio::sync::mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>,
) -> Vec<ServedRoute> {
    std::iter::from_fn(|| gateway_rx.try_recv().ok())
        .filter_map(|msg| match msg {
            xai_acp_lib::AcpClientMessage::ExtNotification(args)
                if args.request.method.as_ref() == "x.ai/session_notification" =>
            {
                serde_json::from_str::<crate::extensions::notification::SessionNotification>(
                    args.request.params.get(),
                )
                .ok()
            }
            _ => None,
        })
        .filter_map(|notification| match notification.update {
            crate::extensions::notification::SessionUpdate::ModelServed { route } => Some(route),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn served_route_reaches_session_update_meta() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, mut gateway_rx) = build_actor().await;
            let metadata = |model: &str| crate::sampling::ResponseModelMetadata {
                served_route: Some(route(model)),
                ..Default::default()
            };

            actor.handle_model_metadata_update(metadata("gpt-5.6-sol")).await;
            actor.handle_model_metadata_update(metadata("gpt-5.6-sol")).await;
            actor.handle_model_metadata_update(metadata("gpt-5.6-luna")).await;

            assert_eq!(
                served_routes_on_wire(&mut gateway_rx),
                vec![route("gpt-5.6-sol"), route("gpt-5.6-luna")],
                "each route change is announced once; a repeat is not"
            );
            assert_eq!(
                actor.build_session_info().await.served_route,
                Some(route("gpt-5.6-luna")),
                "/session reports the latest served route"
            );
        })
        .await;
}
