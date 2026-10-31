//! Mid-turn interjection handling for `SessionActor`: buffer type, formatting, broadcast, and drain.
//! Also hosts `inject_synthetic_user_message`, the shared synthetic-user-message injector the permission-panel followup path reuses.

use super::*;

// Buffer, entry type, and formatting live in the shared xai-interjection-core crate so the server-side agent loop can adopt the same behaviour.
// The shell keeps arrival (ACP ext methods), persistence, and pager echo.
// Re-exported for `acp_session.rs`, which does `pub(crate) use interjection::*;`.
#[allow(unused_imports)]
pub(crate) use xai_interjection_core::{
    INTERRUPT_NOTE, InterjectionBuffer, drain_formatted, format_interjection, frame_user_turn,
};

/// Shell instantiation of the shared entry type: images are ACP content.
pub(crate) type PendingInterjection = xai_interjection_core::PendingInterjection<acp::ImageContent>;

/// Prompt-id prefix for interjections that missed their turn and were converted into standalone prompt turns.
/// The prefix keeps the turn's user echo persist-only.
/// Every pane already rendered the text from the `x.ai/session/interjection` broadcast, so a live echo would duplicate it.
pub(crate) const INTERJECT_FALLBACK_PROMPT_PREFIX: &str = "interject-fallback-";

pub(crate) fn is_interject_fallback(prompt_id: &str) -> bool {
    prompt_id.starts_with(INTERJECT_FALLBACK_PROMPT_PREFIX)
}

impl SessionActor {
    /// An interjection is only merged into a *running* turn (`drain_pending_interjections`).
    /// One that arrives while the session is idle, or lands after the running turn's final drain, would sit in `pending_interjections` forever.
    /// The user's message would be silently lost (the pager already rendered it and said "Interjection sent").
    pub(super) async fn queue_interjection_fallback_prompt(
        &self,
        text: String,
        images: Vec<acp::ImageContent>,
        id: Option<String>,
        front: bool,
    ) {
        let identity = id.map(|id| xai_grok_sampling_types::InterjectionIdentity {
            id,
            display_text: text.clone(),
        });
        let prompt_id = format!("{INTERJECT_FALLBACK_PROMPT_PREFIX}{}", uuid::Uuid::now_v7());
        let mut prompt_blocks = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
        prompt_blocks.extend(images.into_iter().map(acp::ContentBlock::Image));
        // Respect an active plan mode: the interjection was aimed at a turn that ran under it, so its fallback turn must not escape the gate
        let prompt_mode = if self.plan_mode.lock().is_active() {
            crate::session::plan_mode::PromptMode::Plan
        } else {
            crate::session::plan_mode::PromptMode::Agent
        };
        let (respond_to, _) = tokio::sync::oneshot::channel();
        // User message (skips queue_input); invalidate in-flight recap now.
        self.invalidate_side_calls_for_new_prompt();
        let item = InputItem {
            prompt_id,
            prompt_blocks,
            prompt_mode,
            trace_gcs_config: None,
            artifact_tracker: None,
            client_identifier: None,
            screen_mode: None,
            verbatim: false,
            json_schema: None,
            input_origin: InputOrigin::new(super::super::PromptOrigin::User)
                .with_interjection(identity),
            task_wake_fallback: None,
            tool_overrides_update: None,
            respond_to,
            persist_ack: None,
            parsed_prompt_tx: None,
            initial_child_prompt_ready: None,
            queue_meta: None,
            queue_mutation_policy: QueueMutationPolicy::hidden(),
            // Send-now placement (see doc): a later real send-now must not leapfrog this fallback in `queue_input`'s FIFO scan
            send_now: front,
            traceparent: None,
        };
        let mut state = self.state.lock().await;
        if front {
            // Never displace a running front (see doc): insert after it when the front row is the in-flight turn's own item
            let insert_at = usize::from(matches!(
                (state.pending_inputs.front(), state.running_prompt_id()),
                (Some(front_item), Some(running)) if front_item.prompt_id == running
            ));
            state.pending_inputs.insert(insert_at, item);
        } else {
            state.pending_inputs.push_back(item);
        }
        tracing::info!("Converted stranded interjection into a queued prompt turn");
    }

    /// Convert interjections that missed their turn's final drain into queued prompt turns, front of the queue in original order.
    /// Returns the count.
    pub(super) async fn flush_stranded_interjections(&self) -> usize {
        let stranded = self.pending_interjections.drain_all();
        let count = stranded.len();
        // Reversed push_fronts keep entry 0 front-most.
        for entry in stranded.into_iter().rev() {
            self.queue_interjection_fallback_prompt(entry.text, entry.attachments, entry.id, true)
                .await;
        }
        count
    }
    /// Normalize interjection images for injection (shared pipeline above); notices append to `wrapped` (TEXT side only).
    /// Returns the images to attach structurally.
    /// Sessions whose template rejects inline images instead transcribe normalized survivors into the text via the describe pipeline.
    pub(super) async fn prepare_interjection_images(
        &self,
        wrapped: &mut String,
        images: Vec<acp::ImageContent>,
    ) -> Vec<acp::ImageContent> {
        if images.is_empty() {
            return images;
        }
        let is_cursor = self.is_cursor_harness();
        let images = self
            .normalize_images_with_notices(wrapped, images, is_cursor)
            .await;
        if !is_cursor {
            // The `<image_files>` block is the only durable record of where these images live.
            // An interjection still goes through when the assets dir cannot be written.
            if !images.is_empty() {
                let persisted =
                    crate::session::persistence::ensure_owner_only_session_dir(&self.session_info)
                        .and_then(|session_dir| {
                            crate::session::image_describe::persist_and_prepend_image_files(
                                &session_dir,
                                &images,
                                wrapped.as_str(),
                            )
                        });
                match persisted {
                    Ok(with_files) => *wrapped = with_files,
                    Err(error) => tracing::warn!(
                        ?error,
                        "interjection images not saved to assets; sending without <image_files>"
                    ),
                }
            }
            return images;
        }
        if !images.is_empty() {
            match self.transcribe_user_images(wrapped.clone(), &images).await {
                Ok(new_text) => *wrapped = new_text,
                Err(e) => {
                    tracing::warn!(?e, "interjection image processing failed; dropping images");
                    wrapped.push_str(
                        "\n\n[Note: the user attached image(s) to this message, but they could \
                         not be processed in this session and were dropped.]",
                    );
                }
            }
        }
        Vec::new()
    }

    /// Broadcast a mid-turn interjection to every attached client.
    /// The originator uses `id` to claim its optimistic prompt block; other clients render the notification normally.
    pub(super) fn broadcast_interjection(&self, text: &str, id: Option<&str>) {
        let delivery = self.current_actions.delivery_role();
        self.broadcast_interjection_with_delivery(text, id, &delivery);
    }

    pub(super) fn broadcast_interjection_with_delivery(
        &self,
        text: &str,
        id: Option<&str>,
        delivery: &crate::extensions::current_actions::DeliveryRole,
    ) {
        let mut meta = serde_json::Map::new();
        self.stamp_current_actions_version(&mut meta);
        if let Err(error) = crate::extensions::current_actions::stamp_delivery(&mut meta, delivery) {
            self.notifications.gateway.fail_publication();
            tracing::error!(%error, "interjection delivery serialization failed");
            return;
        }
        let mut payload = serde_json::json!({
            "sessionId": self.session_info.id.0.as_ref(),
            "text": text,
            "meta": meta,
        });
        if let Some(id) = id
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("interjectionId".into(), serde_json::json!(id));
        }
        if let Ok(params) = serde_json::value::to_raw_value(&payload) {
            self.notifications
                .gateway
                .forward_fire_and_forget(acp::ExtNotification::new(
                    "x.ai/session/interjection",
                    params.into(),
                ));
        }
    }

    /// Persist and optionally notify the pager for a synthetic user message.
    pub(super) async fn persist_synthetic_user_message(
        &self,
        text: &str,
        notify_pager: bool,
        images: &[acp::ImageContent],
    ) {
        let model_id = self.current_model_id().await;
        self.persist_synthetic_user_message_with_model(text, None, images, &model_id);

        // Notify pager (skipped for interjections; the pager has a local block)
        if notify_pager {
            let user_chunk_meta = serde_json::json!({ "modelId": model_id })
                .as_object()
                .cloned();
            self.send_update(
                acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
                        text.to_string(),
                    )))
                    .meta(user_chunk_meta),
                ),
                None,
            )
            .await;
        }
    }

    /// Persistence half of `persist_synthetic_user_message`, synchronous (model id.
    /// pre-fetched) so the interjection drain can keep the batch submit and its.
    /// persisted user chunks on the same side of any cancellation point.
    pub(super) fn persist_synthetic_user_message_with_model(
        &self,
        text: &str,
        interjection_display_text: Option<&str>,
        images: &[acp::ImageContent],
        model_id: &str,
    ) {
        self.persist_interjection_user_message_with_model(
            text, interjection_display_text, images, model_id, None,
        );
    }

    fn persist_interjection_user_message_with_model(
        &self,
        text: &str,
        interjection_display_text: Option<&str>,
        images: &[acp::ImageContent],
        model_id: &str,
        identity: Option<&xai_grok_sampling_types::InterjectionIdentity>,
    ) {
        let mut user_chunk_meta = serde_json::Map::new();
        if let Some(identity) = identity {
            user_chunk_meta.insert("interjectionIdentity".into(), serde_json::json!(identity));
        }
        user_chunk_meta.insert("modelId".into(), serde_json::json!(model_id));
        let mut text_block = acp::TextContent::new(text.to_string());
        if let Some(display_text) = interjection_display_text {
            user_chunk_meta.insert(
                crate::session::storage::INTERJECTION_META_KEY.into(),
                serde_json::json!(true),
            );
            let mut text_meta = serde_json::Map::new();
            text_meta.insert("displayText".into(), serde_json::json!(display_text));
            text_block = text_block.meta(Some(text_meta));
        }
        let user_chunk_meta = Some(user_chunk_meta);

        // Persist to updates.jsonl: one UserMessageChunk per content block (text first, then any images; Image chunks already round-trip)
        let mut content_blocks = vec![acp::ContentBlock::Text(text_block)];
        content_blocks.extend(images.iter().cloned().map(acp::ContentBlock::Image));
        let notification_meta = self.build_notification_meta();
        for content_block in content_blocks {
            let update = acp::SessionUpdate::UserMessageChunk(
                acp::ContentChunk::new(content_block).meta(user_chunk_meta.clone()),
            );
            let _ = self
                .notifications
                .persistence_tx
                .send(PersistenceMsg::Update(SessionUpdate::Acp(Box::new(
                    acp::SessionNotification::new(self.session_info.id.clone(), update)
                        .meta(notification_meta.clone().as_object().cloned()),
                ))));
        }
    }

    /// Inject a synthetic user message: persist, optionally notify pager, and push to the conversation as a standalone synthetic user message.
    pub(super) async fn inject_synthetic_user_message(
        &self,
        text: &str,
        item: ConversationItem,
        notify_pager: bool,
        images: &[acp::ImageContent],
    ) {
        self.persist_synthetic_user_message(text, notify_pager, images)
            .await;
        self.chat_state_handle.push_user_message(item);
    }

    /// Expand skill slash references in interjection text into the `<skill_information>` envelope (loaded and substituted SKILL.md bodies).
    /// Interjections bypass turn-start slash resolution (`slash_commands::resolve`).
    /// Without this, a queued `/skill` row force-sent mid-turn, or a typed `/skill` interjection, reaches the model as a bare, unexpanded slash command.
    async fn interjection_skill_information(&self, text: &str) -> Option<String> {
        // Mirror turn-start gating (`parse_slash_prefix`): only a leading slash invokes skills
        // "don't run /commit yet" is steering text, not an invocation
        if !text.trim_start().starts_with('/') {
            return None;
        }
        let slash_skills = self.slash_skills_for_resolve().await;
        // Availability without `command_availability()`'s goal-reconciliation side effects; this runs mid-turn inside the drain
        let tool_names = self.registered_tool_names().await;
        let has_workflow_runs = !self.workflow_tracker().await.lock().list().is_empty();
        let availability = self.build_command_availability(&tool_names, has_workflow_runs);
        let parsed = slash_commands::parse_skill_references(text, &slash_skills, availability)?;
        // Deliberately lighter telemetry than turn start: no `skill.activated` span, `PluginUsed`, or `active_skill` stamp
        // Those attribute the turn, which this skill did not start
        // `SkillDispatched` still carries `plugin_source`, so dispatch counts stay complete
        for sk in &parsed {
            xai_grok_telemetry::session_ctx::log_event(
                xai_grok_telemetry::events::SlashCommandUsed {
                    command: sk.name.clone(),
                    args_provided: !sk.args.is_empty(),
                },
            );
            let skill_source =
                crate::session::telemetry::skill_source(sk.scope, sk.plugin_name.as_deref());
            xai_grok_telemetry::session_ctx::log_event(
                xai_grok_telemetry::events::SkillDispatched {
                    skill_name: sk.name.clone(),
                    plugin_source: sk.plugin_name.clone(),
                    trigger: xai_grok_telemetry::events::SkillTrigger::SlashCommand,
                    skill_source: Some(skill_source.to_owned()),
                    skill_origin: sk.origin.clone(),
                },
            );
        }
        slash_commands::build_skill_information_for_refs(
            &parsed,
            &slash_skills,
            &self.session_id_string(),
        )
        .await
    }

    /// When follow-up behavior is Steer, promote held queue rows into interjections, then drain.
    /// Call after a tool batch, at loop top, and before the turn returns to the user.
    /// Returns `true` if any interjections were drained (caller may `continue` so the model sees them next).
    pub(super) async fn drain_interjections_at_safe_point(&self) -> bool {
        // Queue (default) must not re-parse config on every tool, model, or turn-end drain
        // `follow_up_steer_enabled` is mtime-keyed on config.toml, so a live pager settings write is visible without restarting the shell agent
        // The pager is a separate process; an unchanged mtime is a cheap stat
        if crate::util::config::follow_up_steer_enabled().await {
            let has_held = {
                let state = self.state.lock().await;
                let running = state.running_prompt_id();
                // Only editable human rows are promotable; protected pins and queue-hidden fallbacks must not trigger steer promotion
                running.is_some()
                    && state.pending_inputs.iter().any(|item| {
                        item.is_queue_editable() && Some(item.prompt_id.as_str()) != running
                    })
            };
            if has_held {
                self.promote_queued_as_interjections().await;
            }
        }
        self.drain_admitted_messages_at_safe_point().await
    }

    /// Drain already-admitted human interjections and parent Steers.
    /// Does not promote held queue rows; use after turn-end bookkeeping so
    /// late admissions still inject into the running turn.
    pub(super) async fn drain_admitted_messages_at_safe_point(&self) -> bool {
        let drained_human = self.drain_pending_interjections().await;
        let drained_parent = self.drain_parent_messages_at_safe_point().await;
        let drained_children = self.drain_subagent_wakes_at_safe_point().await;
        let drained_receiver = self.drain_durable_deliveries_at_safe_point().await;
        drained_parent || drained_human || drained_children || drained_receiver
    }

    pub(super) async fn drain_pending_interjections(&self) -> bool {
        // Manual drain (not `drain_formatted`): skill parsing needs the raw text.
        // Parsed after wrapping, the envelope's closing `</user_query>` tag would pollute the trailing skill's args.
        // The guard owns the drained entries until the batch is submitted: every await below is a.
        let guard = RestoreOnCancel {
            buffer: self.pending_interjections.clone(),
            entries: self.pending_interjections.drain_all(),
        };
        if guard.entries.is_empty() {
            return false;
        }

        let mut prepared = Vec::with_capacity(guard.entries.len());
        for PendingInterjection { id, text, attachments } in &guard.entries {
            // The sanitizer rewrites `[Image #N: <path>]` to `[Image #N]` before the text reaches the model
            // It covers legacy-client raw text AND text harvested from queued rows sent as interjections
            // Wrapping and truncation stay in the shared crate (`format_interjection`)
            let sanitized = crate::session::placeholder_images::strip_paths_from_image_placeholders(
                text.clone(),
            );
            let skill_information = self.interjection_skill_information(&sanitized).await;
            let mut wrapped = format_interjection(sanitized.clone());
            // The pipeline consumes a clone; the guard keeps the original attachments restorable
            let images = self
                .prepare_interjection_images(&mut wrapped, attachments.clone())
                .await;
            // Model-visible text: <skill_information> follows the wrapped <user_query>, the same order as turn-start prompt assembly
            // It is appended after the image pipeline so the template-specific transcription rewrite cannot mangle the envelope
            // The persisted user chunk stays envelope-only (no SKILL.md body); the typed text rides in `displayText` for replay
            let model_text = match &skill_information {
                Some(skill_information) => {
                    tracing::info!("expanded skill references in mid-turn interjection");
                    format!("{wrapped}\n{skill_information}")
                }
                None => wrapped.clone(),
            };
            let identity = id.as_ref().map(|id| xai_grok_sampling_types::InterjectionIdentity {
                id: id.clone(),
                display_text: sanitized.clone(),
            });
            let mut item = ConversationItem::interjection(model_text);
            if let ConversationItem::User(user) = &mut item {
                user.interjection = identity.clone();
            }
            for img in &images {
                item.add_image(pick_user_image_url(img));
            }
            prepared.push((wrapped, sanitized, images, identity, item));
        }
        // Last await before the submit; from here persistence is deliberately synchronous so no
        // cancellation point can separate the submitted batch from its persisted user chunks
        let model_id = self.current_model_id().await;
        let (persist_parts, chat_items): (Vec<_>, Vec<_>) = prepared
            .into_iter()
            .map(|(wrapped, typed, images, identity, item)| ((wrapped, typed, images, identity), item))
            .unzip();
        if self
            .chat_state_handle
            .try_push_user_messages_batch(chat_items)
            .is_err()
        {
            // The guard restores the entries so `flush_stranded_interjections` can still deliver them
            tracing::error!(
                session_id = %self.session_info.id.0,
                "interjection drain skipped: chat-state actor unavailable"
            );
            return false;
        }
        // Persist only after the submit succeeded: on the failure/cancel paths the entries go back
        // to the buffer and the fallback-prompt turn persists them, so persisting here too would
        // duplicate the user chunks in updates.jsonl
        for (wrapped, typed, images, identity) in persist_parts {
            self.persist_interjection_user_message_with_model(
                &wrapped,
                Some(&typed),
                &images,
                &model_id,
                identity.as_ref(),
            );
        }
        guard.defuse();
        tracing::info!("Injected mid-turn interjections as standalone synthetic user messages");
        // An interjection never cancels the turn, so it leaves no marker on the next user turn (that field is reserved for fatal aborts)
        // The interjection itself is recorded at enqueue time via `Event::Interjected` (carrying the shared `redirect_kind`)
        true
    }
}

/// Cancel-safety guard for `drain_pending_interjections`: a turn abort drops the drain future at one of its awaits (skill resolution, image pipeline, model-id fetch).
/// Entries already drained but not yet submitted to chat state would vanish — `flush_stranded_interjections` would find an empty buffer with nothing to convert into fallback prompts.
/// On drop, unsubmitted entries go back to the front of the buffer, ahead of anything pushed since, keeping arrival order.
#[must_use]
struct RestoreOnCancel {
    buffer: InterjectionBuffer<acp::ImageContent>,
    entries: Vec<PendingInterjection>,
}

impl RestoreOnCancel {
    /// The batch was submitted; the entries are chat state's responsibility now.
    fn defuse(mut self) {
        self.entries.clear();
    }
}

impl Drop for RestoreOnCancel {
    fn drop(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        tracing::warn!(
            count = self.entries.len(),
            "interjection drain aborted before submit; restoring entries for a later drain or flush"
        );
        self.buffer.restore_front(std::mem::take(&mut self.entries));
    }
}

#[cfg(test)]
mod publication_binding_tests {
    use super::*;
    use crate::extensions::current_actions::{DeliveryRole, DELIVERY_META_KEY};
    use crate::extensions::interject::capture_publication;
    use crate::session::current_actions::SnapshotBinding;

    #[tokio::test]
    async fn command_echo_keeps_original_capture_after_same_incarnation_attach() {
        tokio::task::LocalSet::new().run_until(async {
            let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
            let (persistence_tx, _persistence_rx) = tokio::sync::mpsc::unbounded_channel();
            let actor = crate::session::acp_session::support::create_test_actor(
                0, 256_000, 85, gateway_tx, persistence_tx,
            ).await;
            let sid = actor.session_info.id.clone();
            let reader = actor.current_actions.reader(Default::default(), sid.0.to_string());
            let old = SnapshotBinding {
                session_id: sid.0.to_string(), binding_epoch: 1, load_generation: 1,
            };
            assert!(capture_publication(&sid, &old, Some(&reader)).is_err());
            assert!(capture_publication(&sid, &old, None).is_err());
            let original = reader.bind_publication(old.clone()).unwrap();
            let delivery = capture_publication(&sid, &old, Some(&reader)).unwrap();
            let command = crate::session::SessionCommand::Interject {
                text: "original payload".into(), id: Some("original-id".into()),
                images: Vec::new(), delivery,
            };
            let new = SnapshotBinding { load_generation: 2, ..old.clone() };
            let attached = reader.bind_publication(new.clone()).unwrap();
            assert_eq!(original.incarnation, attached.incarnation);
            assert!(capture_publication(&sid, &old, Some(&reader)).is_err());
            let foreign = SnapshotBinding { session_id: "foreign".into(), ..new.clone() };
            assert!(capture_publication(&sid, &foreign, Some(&reader)).is_err());
            tokio::task::yield_now().await;
            let crate::session::SessionCommand::Interject { text, id, images, delivery } = command else {
                panic!("original Interject command");
            };
            assert!(images.is_empty());
            actor.broadcast_interjection_with_delivery(&text, id.as_deref(), &delivery);
            let current = capture_publication(&sid, &new, Some(&reader)).unwrap();
            actor.broadcast_interjection_with_delivery("new payload", Some("new-id"), &current);
            actor.broadcast_interjection_with_delivery("legacy", None, &DeliveryRole::Unavailable);
            let mut echoes = Vec::new();
            while let Ok(message) = gateway_rx.try_recv() {
                if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message
                    && args.request.method.as_ref() == "x.ai/session/interjection"
                {
                    echoes.push(serde_json::from_str::<serde_json::Value>(args.request.params.get()).unwrap());
                }
            }
            assert_eq!(echoes.len(), 3);
            assert_eq!(echoes[0]["text"], "original payload");
            assert_eq!(echoes[0]["interjectionId"], "original-id");
            assert_eq!(echoes[0]["meta"][DELIVERY_META_KEY], serde_json::to_value(DeliveryRole::Live { context: original }).unwrap());
            assert_eq!(echoes[1]["text"], "new payload");
            assert_eq!(echoes[1]["interjectionId"], "new-id");
            assert_eq!(echoes[1]["meta"][DELIVERY_META_KEY], serde_json::to_value(DeliveryRole::Live { context: attached }).unwrap());
            assert_eq!(echoes[2]["meta"][DELIVERY_META_KEY], serde_json::to_value(DeliveryRole::Unavailable).unwrap());
        }).await;
    }
}
