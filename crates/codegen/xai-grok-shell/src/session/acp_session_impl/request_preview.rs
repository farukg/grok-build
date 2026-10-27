use super::*;
use crate::extensions::request_preview::{
    PreviewDelivery, PreviewInput, PreviewInputPosition, PreviewKey, PreviewLateBound,
    PreviewSnapshot, PreviewUnavailable, RequestPreview, RequestPreviewRequest,
};
use crate::session::helpers::context_policy_store::ContextPreviewUnavailable;
use xai_grok_sampler::request_preview::NativePreviewFailure;

#[cfg(test)]
#[path = "request_preview_tests.rs"]
mod tests;

struct NoPreviewSchemaRetrieval;

impl jsonschema::Retrieve for NoPreviewSchemaRetrieval {
    fn retrieve(
        &self, _uri: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "preview cannot retrieve schema resources").into())
    }
}

impl SessionActor {
    pub(crate) async fn preview_next_request(&self, request: RequestPreviewRequest) -> RequestPreview {
        match self.build_next_request_preview(&request).await {
            Ok((snapshot, latebound)) => {
                let mut latebound = latebound.into_iter();
                match latebound.next() {
                    Some(first) => RequestPreview::Partial { snapshot, first, additional: latebound.collect() },
                    None => RequestPreview::CompleteAsOf { snapshot },
                }
            }
            Err(cause) => RequestPreview::unavailable(&request, cause),
        }
    }

    async fn build_next_request_preview(
        &self, request: &RequestPreviewRequest,
    ) -> Result<(PreviewSnapshot, Vec<PreviewLateBound>), PreviewUnavailable> {
        if request.session_id != self.session_info.id.0.as_ref() {
            return Err(PreviewUnavailable::BindingChanged);
        }
        let mut latebound = vec![PreviewLateBound::ScheduledInjections, PreviewLateBound::ModelConfigurationRefresh];
        let (active, queued, active_schema) = {
            let state = self.state.lock().await;
            let running = state.running_prompt_id();
            let active_schema = running.and_then(|id| state.pending_inputs.iter().find(|item| item.prompt_id == id))
                .map(|item| item.json_schema.clone());
            let queued = state.pending_inputs.iter().find(|item| Some(item.prompt_id.as_str()) != running)
                .map(|item| (
                    item.prompt_blocks.clone(), item.verbatim, item.send_now, item.json_schema.clone(),
                    matches!(item.input_origin.as_prompt_origin(), crate::session::PromptOrigin::User),
                    item.tool_overrides_update.is_some(), item.prompt_mode,
                ));
            (running.is_some(), queued, active_schema)
        };
        if active {
            latebound.extend([PreviewLateBound::ActiveWork, PreviewLateBound::UpcomingToolResults]);
            if active_schema.is_none() { latebound.push(PreviewLateBound::ActiveStructuredOutput); }
        }
        let plan_active = self.plan_mode.lock().is_active();
        let selected_mode = *self.current_prompt_mode.lock();
        if plan_active || selected_mode == crate::session::plan_mode::PromptMode::Plan {
            latebound.push(PreviewLateBound::PlanReminders);
        }
        if (selected_mode == crate::session::plan_mode::PromptMode::Plan) != plan_active {
            latebound.push(PreviewLateBound::PromptModePreparation);
        }
        if self.memory.is_enabled() && !self.memory.context_injected.load(std::sync::atomic::Ordering::Relaxed) {
            latebound.push(PreviewLateBound::FirstTurnMemory);
        }
        let mut appended = Vec::new();
        let pending = self.pending_interjections.snapshot();
        if !pending.is_empty() {
            latebound.push(PreviewLateBound::PendingInterjections);
            if active {
                for entry in pending {
                    if !entry.attachments.is_empty() {
                        latebound.push(PreviewLateBound::AttachmentPreparation);
                    } else if entry.text.trim_start().starts_with('/') {
                        latebound.push(PreviewLateBound::SkillOrCommandExpansion);
                    } else {
                        let text = crate::session::placeholder_images::strip_paths_from_image_placeholders(entry.text);
                        appended.push(ConversationItem::interjection(xai_interjection_core::format_interjection(text)));
                    }
                }
            }
        }
        let (position, schema) = match &request.input {
            PreviewInput::Current => {
                if active {
                    if queued.is_some() { latebound.push(PreviewLateBound::QueuePromotion); }
                    (PreviewInputPosition::Current, active_schema.clone().flatten())
                } else if let Some((blocks, verbatim, send_now, schema, human, overrides, mode)) = queued {
                    latebound.push(PreviewLateBound::QueuePromotion);
                    if mode != selected_mode { latebound.push(PreviewLateBound::PromptModePreparation); }
                    if mode == crate::session::plan_mode::PromptMode::Plan { latebound.push(PreviewLateBound::PlanReminders); }
                    if overrides { latebound.push(PreviewLateBound::ToolOverridePromotion); }
                    if human {
                        self.append_preview_prompt(&blocks, verbatim, send_now, &mut appended, &mut latebound);
                    }
                    (PreviewInputPosition::QueuedInput, schema)
                } else {
                    (PreviewInputPosition::Current, None)
                }
            }
            PreviewInput::Draft { delivery, blocks, json_schema } => {
                match delivery {
                    PreviewDelivery::Prompt | PreviewDelivery::Queue if active || queued.is_some() => {
                        latebound.push(PreviewLateBound::QueuePromotion);
                        let schema = if active {
                            active_schema.clone().flatten()
                        } else if let Some((blocks, verbatim, send_now, schema, human, overrides, mode)) = queued.as_ref() {
                            if *human { self.append_preview_prompt(blocks, *verbatim, *send_now, &mut appended, &mut latebound); }
                            if *overrides { latebound.push(PreviewLateBound::ToolOverridePromotion); }
                            if *mode != selected_mode { latebound.push(PreviewLateBound::PromptModePreparation); }
                            schema.clone()
                        } else {
                            None
                        };
                        (PreviewInputPosition::BehindQueuedInput, schema)
                    }
                    PreviewDelivery::Interjection if !active => {
                        latebound.push(PreviewLateBound::QueuePromotion);
                        (PreviewInputPosition::DeferredInterjection, None)
                    }
                    PreviewDelivery::Interjection => {
                        let (text, images) = crate::extensions::content::split_content(blocks.clone());
                        if !images.is_empty() {
                            latebound.push(PreviewLateBound::AttachmentPreparation);
                        } else if let Some(text) = text {
                            if text.trim_start().starts_with('/') {
                                latebound.push(PreviewLateBound::SkillOrCommandExpansion);
                            } else {
                                let text = crate::session::placeholder_images::strip_paths_from_image_placeholders(text);
                                appended.push(ConversationItem::interjection(xai_interjection_core::format_interjection(text)));
                            }
                        }
                        (PreviewInputPosition::Draft { delivery: *delivery }, active_schema.clone().flatten())
                    }
                    PreviewDelivery::Prompt | PreviewDelivery::Queue | PreviewDelivery::SendNow => {
                        self.append_preview_prompt(blocks, false, matches!(delivery, PreviewDelivery::SendNow), &mut appended, &mut latebound);
                        (PreviewInputPosition::Draft { delivery: *delivery }, json_schema.clone())
                    }
                }
            }
        };
        let definitions = self.prepare_tool_definitions_inner().await;
        let mut tools = self.effective_function_tools(&definitions).await;
        if !self.startup_hints.is_subagent {
            tools.push(super::stop_gate::wait_tool_spec());
        }
        let store = self.context_policy_binding.authority.read().await;
        let (selection, policy) = store.preview_snapshot(
            &request.session_id, &self.context_policy_binding.incarnation,
        ).map_err(|cause| match cause {
            ContextPreviewUnavailable::AuthorityNotLoaded => PreviewUnavailable::AuthorityNotLoaded,
            ContextPreviewUnavailable::LegacyNotInspected => PreviewUnavailable::LegacyPolicyNotInspected,
        })?;
        let preview = self.chat_state_handle.preview_request_with_context(
            if active { xai_chat_state::PreviewHistory::InFlight } else { xai_chat_state::PreviewHistory::Stable },
            policy, appended, tools, None,
        ).await.map_err(|cause| match cause {
            xai_chat_state::PreviewFailure::MailboxClosed => PreviewUnavailable::ChatMailboxClosed,
            xai_chat_state::PreviewFailure::ReplyLost => PreviewUnavailable::ChatReplyLost,
        })?;
        drop(store);
        let config = preview.sampling_config;
        let history_bytes = serde_json::to_vec(&preview.canonical_history)
            .map_err(|_| PreviewUnavailable::Serialization)?;
        let key = PreviewKey {
            session_id: request.session_id.clone(), binding: selection.version.incarnation,
            canonical_history_revision: *blake3::hash(&history_bytes).as_bytes(),
            context_policy_revision: selection.version.revision, ui_generation: request.ui_generation,
        };
        let context_policy = preview.built.context_policy;
        let mut candidate = preview.built.request;
        // Preview has no admission, request, or attempt identity, including header-only IDs.
        candidate.x_grok_conv_id = None;
        candidate.x_grok_req_id = None;
        if context_policy.includes(xai_grok_sampling_types::ContextCategory::ToolDefinitions) {
            candidate.hosted_tools = self.hosted_tools_for_turn();
            if let Some(schema) = schema {
                if jsonschema::options().with_retriever(NoPreviewSchemaRetrieval).build(&schema).is_err() {
                    return Err(PreviewUnavailable::SchemaNotPrepared);
                }
                if config.api_backend.supports_native_schema() {
                    candidate.json_schema = Some(schema);
                } else {
                    candidate.tools.push(super::turn::structured_output_tool_spec(schema));
                }
            }
        }
        candidate.max_output_tokens = self.tool_context.clamp_task_model_request(candidate.max_output_tokens)
            .map_err(|_| PreviewUnavailable::OutputBudget)?;
        let membership = candidate.projected_membership();
        let projected_items = candidate.items.clone();
        let native = self.sampler_handle.preview_request(candidate).await.map_err(|cause| match cause {
            NativePreviewFailure::MailboxClosed => PreviewUnavailable::SamplerMailboxClosed,
            NativePreviewFailure::ReplyLost => PreviewUnavailable::SamplerReplyLost,
            NativePreviewFailure::ModelNotInitialized => PreviewUnavailable::ModelNotInitialized,
            NativePreviewFailure::ModelConfigurationChanged => PreviewUnavailable::ModelConfigurationChanged,
            NativePreviewFailure::Serialization => PreviewUnavailable::Serialization,
        })?;
        if native.backend != config.api_backend { return Err(PreviewUnavailable::ModelConfigurationChanged); }
        Ok((PreviewSnapshot {
            key, context_policy, input_position: position, membership, native, projected_items,
            working_directory: self.tool_context.cwd.to_path_buf(),
        }, latebound))
    }

    fn append_preview_prompt(
        &self, blocks: &[acp::ContentBlock], verbatim: bool, send_now: bool,
        appended: &mut Vec<ConversationItem>, latebound: &mut Vec<PreviewLateBound>,
    ) {
        if Self::extract_bash_command(blocks).is_some()
            || crate::session::slash_authority::parse_slash_prefix(blocks).is_some()
        {
            latebound.push(PreviewLateBound::SkillOrCommandExpansion);
            return;
        }
        use crate::session::prompt_parser::{PreviewPromptDeferred, preview_prompt_text};
        match preview_prompt_text(blocks, verbatim, self.is_cursor_harness(), send_now) {
            Ok(text) => {
                latebound.push(PreviewLateBound::InterruptEnvelope);
                appended.push(ConversationItem::user(text));
            }
            Err(cause) => latebound.push(match cause {
                PreviewPromptDeferred::AttachmentPreparation => PreviewLateBound::AttachmentPreparation,
                PreviewPromptDeferred::FileExpansion => PreviewLateBound::FileExpansion,
                PreviewPromptDeferred::LargePromptOffload => PreviewLateBound::LargePromptOffload,
            }),
        }
    }
}
