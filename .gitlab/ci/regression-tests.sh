#!/usr/bin/env bash
set -euo pipefail

: "${1:?Pinned toolchain is required from build-linux.sh}"
toolchain=$1
mkdir -p dist/regression
sequence=0
: > dist/regression/summary.txt
source .gitlab/ci/test-target.sh

declare -A archived_future_reports=()
archive_future_reports() {
    local evidence=$1 log report_id report_exit report
    local -a logs=()
    for log in "$evidence.compile.log" "$evidence.run.log"; do
        if [[ -f $log ]]; then
            logs+=("$log")
        fi
    done
    if (( ${#logs[@]} == 0 )); then
        return 0
    fi
    awk '
        {
            line = $0
            while (match(line, /cargo report future-incompatibilities --id [0-9]+/)) {
                command = substr(line, RSTART, RLENGTH)
                sub(/^.*--id /, "", command)
                printf "%s\t%s\t%s\n", command, FILENAME, FNR
                line = substr(line, RSTART + RLENGTH)
            }
        }
    ' "${logs[@]}" > "$evidence.future-report-sources.tsv"
    cut -f1 "$evidence.future-report-sources.tsv" | sort -u > "$evidence.future-report-ids"
    printf 'future_report_ids=%s.future-report-ids future_report_sources=%s.future-report-sources.tsv\n' \
        "$evidence" "$evidence" >> dist/regression/summary.txt
    while IFS= read -r report_id; do
        if [[ -n ${archived_future_reports[$report_id]+present} ]]; then
            continue
        fi
        report="dist/regression/future-incompatibilities-id-$report_id"
        report_exit=0
        printf 'cargo --color never report future-incompatibilities --id %s\n' "$report_id" > "$report.command"
        cargo --color never report future-incompatibilities --id "$report_id" > "$report.log" 2>&1 || report_exit=$?
        printf 'exit=%s\n' "$report_exit" > "$report.result"
        printf 'future_report_kind=explicit-id future_report_id=%s future_report_exit=%s future_report_log=%s.log future_report_command=%s.command future_report_result=%s.result\n' \
            "$report_id" "$report_exit" "$report" "$report" "$report" >> dist/regression/summary.txt
        printf 'Future-incompatibility diagnostics: id=%s exit=%s; full context: %s.log\n' "$report_id" "$report_exit" "$report"
        archived_future_reports[$report_id]=$report
    done < "$evidence.future-report-ids"
}

run_tests() {
    local package=$1 target=$2 filter
    shift 2
    if (( $# == 0 )); then
        set -- ''
    fi
    local policy=allow-ignored
    if [[ ${1-} == --all-executed ]]; then
        policy=all-executed
        shift
    fi
    local -a command=(cargo "+$toolchain" -Z checksum-freshness test)
    case "$package" in
        xai-monotonic-clock)
            command+=(-p "$package" --locked --features native)
            ;;
        textwrap)
            # Excluded workspace: CI resolves its own lock, not the CLI lock.
            command+=(--manifest-path third_party/textwrap/Cargo.toml)
            ;;
        xai-grok-pager | xai-grok-shell)
            command+=(-p "$package" --locked --features test-support)
            ;;
        xai-acp-lib | xai-grok-markdown | xai-grok-pager-render | xai-grok-session-events | xai-grok-sampler | xai-tool-runtime | xai-grok-pager-bin | xai-grok-sampling-types | xai-grok-tools | xai-grok-test-support)
            command+=(-p "$package" --locked)
            ;;
        *)
            printf 'Unknown regression package: %s\n' "$package" >&2
            return 2
            ;;
    esac
    case "$target" in
        lib) command+=(--lib) ;;
        *) command+=(--test "$target") ;;
    esac

    sequence=$((sequence + 1))
    local evidence="dist/regression/$sequence-$package-$target"
    printf '\nRegression target: %s / %s\n' "$package" "$target"
    if [[ $package == xai-grok-pager-bin ]]; then
        policy=all-executed
    fi
    local result=0
    run_test_target "$evidence" "$policy" command "$@" || result=$?
    archive_future_reports "$evidence"
    printf 'target=%s package=%s target_name=%s exit=%s command=%s.command result=%s.result\n' \
        "$sequence" "$package" "$target" "$result" "$evidence" "$evidence" >> dist/regression/summary.txt
    if (( result != 0 )); then
        printf 'Regression target %s/%s failed: invocation=%s exit=%s; outcome=%s.result; command=%s.command; diagnostic prefix=%s\n' \
            "$package" "$target" "$sequence" "$result" "$evidence" "$evidence" "$evidence" >&2
        return "$result"
    fi
    if [[ $package == xai-grok-pager-bin ]]; then
        sha256sum "$CARGO_TARGET_DIR/debug/xai-grok-pager" > "$evidence.binary.sha256"
    fi
}

# Keep independent targets diagnostic even when an earlier target fails.
status=0
run_tests xai-grok-markdown lib --all-executed \
    'streaming::source_tests::' \
    'streaming::tests::' \
    'latex_delimiters::tests::' \
    'hyperlinks::hyperlink_tests::' \
    'url_scan::tests::' \
    'output::code_block_span_tests::' || status=$?

run_tests xai-grok-pager-render lib \
    'render::wrapping::tests::' \
    'bounded_prefix_matches_the_existing_full_wrap' \
    'bounded_quote_prefix_matches_full_quote_layout' \
    'bounded_wrap_never_builds_the_unseen_line_tail' \
    'location_path::tests::' \
    'render::tool_paths::tests::' \
    'render::osc8::tests::' || status=$?

run_tests xai-grok-pager lib \
    'app::error_display::tests::retry_schedule_has_known_generic_wire_classification' \
    'views::todo_pane::tests::' \
    'sidebar_cache_tracks_content_status_filter_width_and_style' \
    'app::agent_view::context_sidebar_tests::' \
    'f7_todo_projection_reuses_unchanged_frames_including_folded_resize' \
    'scrollback::blocks::message_symbol::tests::' \
    'unicode_path_link_selection_and_styled_markdown_copy_exclude_chrome' \
    'acp_completed_tools_are_successful_but_orphan_turn_end_is_neutral' \
    'acp_failed_terminal_without_error_text_keeps_failure_color' \
    'confirmed_tool_outcomes_override_running_and_invalidate_cached_paint' \
    'scrollback::render::tests::' \
    'acp::tracker::tests::' \
    'app::acp_handler::tests::progress_lifecycle::' \
    'app::acp_handler::tests::session_events::' \
    'app::acp_handler::tests::terminal_activity_integration::' \
    'app::dispatch::tests::queue_send_now::' \
    'app::dispatch::tests::session::load::activity_source::' \
    'app::effects::session_list::tests::' \
    'app::roster::tests::' \
    'app::dispatch::tests::dashboard' \
    'views::dashboard::state::tests::' \
    'views::dashboard::render::tests::' || status=$?

run_tests xai-grok-pager lib --all-executed \
    'scrollback::scrollback_pane::tests::' \
    'scrollback::wrappers::entry_renderer::tests::' \
    'app::session_views::inline_output::tests::' \
    'app::session_views::inline_output::paint_tests::' \
    'app::session_views::inline_output::completion_tests::' \
    'scrollback::blocks::markdown_content::source_tests::' \
    'scrollback::blocks::markdown_content::tests::' \
    'scrollback::blocks::agent::tests::' \
    'scrollback::blocks::thinking::tests::' \
    'views::block_viewer::tests::' \
    'acp::tracker::tests::whitespace_thought_chunk_is_preserved_until_a_blank_block_is_finished' \
    'acp::tracker::tests::runtime::' \
    'app::acp_handler::tests::terminal_activity_integration::' \
    'app::acp_handler::tests::terminal_activity_integration::terminal_during_reload_cannot_cross_live_and_historical_legacy_owners' \
    'app::acp_handler::tests::terminal_activity_integration::registered_terminal_cannot_confirm_a_drained_legacy_entry_with_the_same_provider' \
    'app::acp_handler::tests::terminal_activity_integration::recycled_legacy_provider_cannot_confirm_either_drained_entry' \
    'app::acp_handler::tests::session_events::compact_auth_failure_transfers_original_prompt_at_its_rpc_response' \
    'app::acp_handler::tests::sampling_lifecycle::' \
    'app::dispatch::tests::auth::' \
    'app::dispatch::tests::session::load::' \
    'app::acp_handler::tests::tool_execution_timing::' \
    'app::acp_handler::tests::tool_execution_timing::frontend_current_actions::' \
    'app::acp_handler::tests::tool_execution_timing::frontend_publication::stale_generation_reply_and_marker_cannot_take_current_load_custody' \
    'app::acp_handler::tests::tool_execution_timing::frontend_publication::missing_role_historical_role_and_foreign_actor_cannot_bootstrap_transition_authority' \
    'app::acp_handler::tests::tool_execution_timing::frontend_publication::explicit_discard_completes_without_row_clock_ack_or_reconnect_cursor' \
    'app::session_load_barrier::' \
    'app::dispatch::tests::queue_send_now::' \
    -- \
    --skip 'app::acp_handler::tests::tool_execution_timing::frontend_publication::actual_resident_prompt_load_and_reconnect_use_pager_marker_before_response' \
    --skip 'app::acp_handler::tests::tool_execution_timing::frontend_publication::actual_transport_only_resident_load_never_claims_pager_settlement' || status=$?

# Resident harnesses require separate processes and must not remain ignored.
run_tests xai-grok-pager lib --all-executed \
    'app::acp_handler::tests::tool_execution_timing::frontend_publication::actual_resident_prompt_load_and_reconnect_use_pager_marker_before_response' \
    -- --exact --ignored || status=$?

run_tests xai-grok-pager lib --all-executed \
    'app::acp_handler::tests::tool_execution_timing::frontend_publication::actual_transport_only_resident_load_never_claims_pager_settlement' \
    -- --exact --ignored || status=$?

run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::cancel::tests::clear_wake_suppression_re_admits_wakes_and_persists_automatic' \
    'session::storage::jsonl::completion::tests::terminal_restart::parent_admission_distinguishes_child_history_hiding_from_explicit_retirement' \
    -- --exact || status=$?

# Preserve complete failures before the broader shell invocation can time out.
run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::cancel_running_task_tests::cancel_resolves_front_when_running_task_is_none' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::cancel_running_task_tests::cancel_running_task_teardown_clears_running_and_pending_work' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::turn_end::durability_tests::pinned_no_task_cancel_waits_for_real_commit_and_rejects_late_completion' \
    'session::acp_session::turn_end::durability_tests::pinned_no_task_cancel_returns_original_write_error_and_preserves_next_rpc' \
    'session::acp_session::turn_end::durability_tests::pinned_no_task_cancel_returns_lost_ack_error_and_preserves_next_rpc' \
    'session::acp_session::turn_end::durability_tests::caller_completion_waits_for_replayable_terminal_commit' \
    'session::acp_session::turn_end::durability_tests::failed_or_lost_terminal_ack_never_returns_caller_success' \
    'session::acp_session::prompt_queue_actor_tests::no_task_cancel_and_promoter_choose_one_owner' \
    'session::acp_session::prompt_queue_actor_tests::dropped_finalization_lease_blocks_production_promotion' \
    'session::acp_session::prompt_queue_actor_tests::stale_rewind_prompt_id_does_not_cancel_promoted_front' \
    'session::acp_session::prompt_queue_actor_tests::claimed_rewind_leaves_queued_next_prompt_untouched' \
    'session::acp_session::prompt_queue_actor_tests::stale_completion_does_not_clear_promoted_turns_running_task' \
    'session::acp_session::turn_completion_emit_tests::cancel_without_running_task_persists_none_elapsed' \
    'session::acp_session::turn_completion_emit_tests::send_now_cancel_in_completion_race_window_still_persists_turn_completed' \
    'session::acp_session::turn_end_reporting_tests::stale_completion_does_not_post_process_the_successor_turn' \
    'session::acp_session::turn_end_reporting_tests::a_completion_arriving_after_its_cancel_reports_nothing' \
    'session::acp_session::cancel_running_task_tests::cancel_running_task_interactive_preserves_queued_work' \
    'session::acp_session::cancel_running_task_tests::cancel_after_own_completion_sweep_preserves_queued_user_prompt' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'agent::mvp_agent::tests::child_prompt_tests::native_wake_admission::completed_native_worker_admits_agent_bootstrap_once' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'agent::mvp_agent::tests::child_prompt_tests::native_wake_admission::completed_native_worker_admits_human_bootstrap_once' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'agent::mvp_agent::tests::child_prompt_tests::native_wake_admission::completed_native_worker_receiver_contention_is_unconfirmed_without_model_input' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::cancel_running_task_tests::cancel_propagates_to_sampler_handle_so_no_further_emission' \
    -- --exact || status=$?

run_tests xai-grok-shell lib --all-executed \
    'agent::mvp_agent::tests::ordered_publication_tests::actual_working_resident_cut_keeps_original_task_and_prompt_rpc' \
    -- --exact || status=$?

run_tests xai-grok-shell lib \
    'agent::update_chunk_merge::progress_tests::' \
    'compaction::admission_tests::' \
    'sampling_events::tests::' \
    'session::acp_session::background_tasks::tests::' \
    'session::acp_session::tool_calls::tool_cancellation::tests::' \
    'session::acp_session::tool_calls::tool_execution_tests::' \
    'session::acp_session::cancel_running_task_tests::' \
    'session::storage::jsonl::history_decoder_tests::' \
    'session::storage::audit::tests::' \
    'session::storage::update_decoder_tests::' \
    'read_chat_history_' \
    'strip_invalid_images_' \
    'agent::mvp_agent::tests::child_prompt_tests::native_wake_admission::' \
    'completed_native_worker_admits_agent_bootstrap_once' \
    'completed_native_worker_admits_human_bootstrap_once' \
    'completed_native_worker_receiver_contention_is_unconfirmed_without_model_input' \
    'transient_retry_loop_tests::' \
    'storage::summary_write::tests::observational_appends_preserve_activity_and_legacy_fallback' \
    'storage::summary_write::tests::loading_identity_does_not_change_activity_or_legacy_fallback' \
    'unified_list::row::tests::list_dtos_preserve_exact_activity_separately_from_effective_updated_time' || status=$?

# Ordered publication and custody repairs must execute every compiled case.
run_tests xai-acp-lib lib --all-executed \
    'publication_tests::' \
    'publication_tests::receiver_settlement_cannot_be_selected_after_sharing_using_or_exporting_the_sender' || status=$?

run_tests xai-grok-shell lib --all-executed \
    'session::acp_session::updates::ordered_publication_tests::' \
    'session::acp_session::updates::ordered_publication_tests::durable_producer_enrollment_survives_allocation_persistence_and_held_receiver' \
    'session::acp_session::updates::ordered_publication_tests::stale_queued_record_needs_actual_receiver_discard_completion' \
    'session::acp_session::updates::ordered_publication_tests::cancelled_durable_producer_cannot_become_a_successful_prefix' \
    'session::acp_session::updates::ordered_publication_tests::cancelled_native_tool_terminal_after_source_change_keeps_publication_debt' \
    'agent::mvp_agent::tests::ordered_publication_tests::' \
    'agent::mvp_agent::tests::ordered_publication_tests::actual_local_cut_waits_for_background_application_despite_overtaking_transport' \
    'agent::mvp_agent::tests::ordered_publication_tests::actual_working_resident_cut_keeps_original_task_and_prompt_rpc' \
    'agent::mvp_agent::tests::ordered_publication_tests::old_generation_cannot_finish_a_newer_load' \
    'agent::mvp_agent::tests::ordered_publication_tests::replaced_resident_actor_cannot_publish_its_old_prepared_end' \
    'agent::mvp_agent::tests::ordered_publication_tests::rejected_actual_history_cannot_be_hidden_by_a_successful_live_cut' \
    'agent::mvp_agent::tests::ordered_publication_tests::failed_receiver_completion_is_not_a_successful_generation_cut' \
    'agent::mvp_agent::tests::ordered_publication_tests::cancelled_load_retains_held_historical_receipt_for_actual_no_replay_load' \
    'agent::mvp_agent::tests::ordered_publication_tests::cancelled_load_retains_already_failed_historical_receipt' \
    'agent::mvp_agent::tests::ordered_publication_tests::transport_only_load_keeps_observation_but_never_claims_receiver_settlement' \
    'agent::mvp_agent::replay_tests::unbound_full_replay_keeps_original_records_without_archived_live_authority' \
    'agent::mvp_agent::replay_tests::unbound_cursor_replay_keeps_original_records_without_archived_live_authority' || status=$?

# Native adoption and completion publication must execute every compiled case.
run_tests xai-grok-shell lib --all-executed \
    'agent::subagent::spawn::native_crash_adoption_tests::' \
    'agent::subagent::spawn::native_crash_adoption_tests::adoption_attempt_teardown::' \
    'agent::subagent::spawn::native_crash_adoption_tests::adoption_explicit_cancel::' \
    'native_released_attempt_end_rejects_backend_stop_after_real_runner_snapshot' \
    'native_released_attempt_stop_before_terminal_decision_is_durably_cancelled' || status=$?

run_tests xai-grok-shell lib --all-executed \
    'tools::notification_bridge::tests::consumed_background::' \
    'available_terminal_burst_uses_one_admission_and_one_trace' \
    'stalled_burst_has_one_shared_admission_budget_and_preserves_every_fallback' \
    'tools::notification_bridge::tests::actor_restart::' \
    'session::acp_session::completion_admission::transaction::task_publication::tests::' \
    'consumed_result_published_after_final_answer_is_ui_only_even_with_a_stale_wait' \
    'jointly_available_outstanding_results_batch_and_duplicate_publication_cannot_wake_again' \
    'consuming_one_queued_batch_member_preserves_its_outstanding_peer' \
    'a_committed_tool_receipt_racing_publication_is_not_a_live_recovery_wake' \
    'an_uncommitted_result_and_speculative_reminder_cache_do_not_consume_the_wake' \
    'outstanding_declared_goal_target_still_wakes_once_and_unselected_peer_stays_silent' \
    'human_stop_parks_the_whole_durable_batch_without_consuming_it' || status=$?

run_tests xai-grok-tools lib --all-executed \
    'reminders::task_completion::tests::' \
    'consumed_result_observation_survives_wake_suppression_and_reservations' \
    'inlined_completion_reminder_carries_its_terminal_receipt_observation' \
    'suppressed_after_get_task_output_completed' \
    'ctrl_c_gate_suppresses_visible_completion_without_reporting_it' \
    'goal_loop_suppresses_ordinary_background_tasks_without_repeating_them' \
    'task_completion_reservations_suppress_reminders' || status=$?

run_tests xai-monotonic-clock lib --all-executed \
    '' \
    'native_clock::tests::matching_process_offset_owner_and_active_thread_prove_one_namespace' \
    'native_clock::tests::contradictory_process_or_thread_namespace_evidence_is_unavailable' \
    'native_clock::tests::coherent_namespace_switches_during_sampling_are_unavailable' \
    'native_clock::tests::actual_linux_worker_thread_uses_the_process_offset_owner' || status=$?

run_tests xai-tool-runtime lib \
    'execution::tests::' || status=$?

run_tests xai-grok-session-events lib \
    'decode::tests::' \
    'decode::tests::entry_accepts_legacy_tool_completion_omissions' \
    'decode::tests::unknown_variants_and_malformed_known_payloads_are_refused' \
    'log::tests::' \
    'log::tests::test_emit_writes_jsonl' \
    'log::tool_execution_tests::' || status=$?

run_tests xai-grok-pager-bin session_audit || status=$?

run_tests xai-grok-sampling-types lib \
    'upgrade_legacy_reasoning' || status=$?

run_tests xai-grok-sampling-types lib --all-executed \
    'error::retry_schedule_cause_tests::instant_range_wire_and_readback' \
    'error::retry_schedule_cause_tests::deadline_missing_runtime_wire_and_readback' \
    'error::retry_schedule_cause_tests::deadline_time_disabled_wire_and_readback' \
    'error::retry_schedule_cause_tests::deadline_rounding_overflow_wire_and_readback' \
    'error::retry_schedule_cause_tests::deadline_tick_out_of_range_wire_and_readback' \
    'error::retry_schedule_cause_tests::timer_shutdown_wire_and_readback' \
    'error::retry_schedule_cause_tests::timer_at_capacity_wire_and_readback' \
    'error::retry_schedule_cause_tests::timer_invalid_wire_and_readback' \
    'error::retry_schedule_cause_tests::missing_detail_is_rejected_without_a_runtime_cause' \
    'error::retry_schedule_cause_tests::wrong_detail_is_rejected_without_a_runtime_cause' \
    'error::retry_schedule_cause_tests::unknown_variants_are_rejected_without_a_runtime_cause' \
    'error::retry_schedule_cause_tests::existing_content_first_and_unit_map_forms_remain_readable' \
    -- --exact || status=$?

run_tests xai-grok-sampler lib \
    'events::tests::error_kind_wire_string_round_trips_for_every_variant' \
    'retry::tests::' \
    'retry_deadline::tests::' \
    'retry_deadline::tests::delayed_delivery_uses_the_original_twenty_minute_deadline' \
    'retry_deadline::tests::native_projection_preserves_a_tokio_controlled_anchor' \
    'retry_deadline::tests::controlled_clock_still_waits_when_native_projection_is_unavailable' \
    'retry_deadline::tests::diagnostic_wall_jumps_do_not_change_the_monotonic_deadline' \
    'retry_deadline::tests::linux_separate_processes_share_the_scheduler_clock' \
    'actor::request_task::tests::' || status=$?

run_tests xai-grok-sampler test_actor \
    'retry_owner_tests::' \
    'retry_owner_tests::transport_then_doom_retries_use_distinct_cumulative_dispatch_owners' \
    'retry_owner_tests::retry_dispatch_is_observable_while_the_server_has_not_produced_a_token' \
    'retry_owner_tests::actual_429_scheduler_keeps_the_entire_twenty_minute_retry_after' || status=$?

# Manifest-only: upstream dev-dependencies are outside the workspace lock.
run_tests textwrap lib \
    'wrap_algorithms::bounded_tests::' \
    'wrap_algorithms::bounded_tests::callback_and_full_collection_keep_known_first_fit_boundaries' \
    'wrap_algorithms::bounded_tests::penalty_is_included_when_deciding_the_boundary' \
    'wrap_algorithms::bounded_tests::break_stops_emission_and_fragment_work_at_one_lookahead' \
    'wrap_algorithms::bounded_tests::empty_input_emits_one_empty_line' \
    'wrap_algorithms::tests::wrap_string_longer_than_f64' \
    'wrap::tests::to_be_or_not' \
    'fill::tests::' \
    'fill::tests::preserve_line_breaks' || status=$?

run_tests xai-grok-test-support lib --all-executed \
    'inference_override::tests::session_matcher_owns_a_dynamic_nominal_session' \
    'inference_override::tests::session_matcher_preserves_endpoint_kind_and_body_conditions' \
    'inference_override::tests::session_matcher_preserves_static_and_unfiltered_builders' \
    -- --exact || status=$?

# These Cargo integration targets own their in-process ACP/mock-inference fixtures.
run_tests xai-grok-shell test_completion_admission || status=$?
run_tests xai-grok-shell test_admission_process_recovery || status=$?
run_tests xai-grok-shell test_completion_goal_activation || status=$?

future_report_exit=0
cargo --color never report future-incompatibilities > dist/regression/future-incompatibilities.log 2>&1 || future_report_exit=$?
printf 'Future-incompatibility diagnostics: exit=%s; full context: dist/regression/future-incompatibilities.log\n' "$future_report_exit"
printf 'future_report_exit=%s\nfuture_report_log=dist/regression/future-incompatibilities.log\n' "$future_report_exit" >> dist/regression/summary.txt
printf 'future_report_kind=default\n' >> dist/regression/summary.txt

printf 'attempted_targets=%s\nexecution_exit=%s\n' "$sequence" "$status" >> dist/regression/summary.txt
exit "$status"
