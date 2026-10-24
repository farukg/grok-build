# Child visibility contract — authored, not verified

## Confirmed source defect

`session/acp_session_impl/parent_message.rs::drain_parent_messages_at_safe_point` admitted Steer through the production delivery lifecycle, persisted ACP UserMessageChunk, and pushed ConversationItem::agent_message to the child sampler, but never forwarded that chunk to the child gateway. PersistenceMsg::Update does not supply the missing live gateway notification. Thus a sender's Accepted/tool output did not prove child-view visibility. Additionally, the batch reused one notification eventId for multiple messages, risking replay/client dedup loss.

The authored fix forwards the SAME notification to the child gateway only after the production delivery commit succeeds. Each projected message gets its own eventId. No Pager-only metadata workaround and no duplicate persistence path were introduced.

## Exact wire / identity contract

- Admission acknowledgement stays ActiveMessageAdmission::Admitted -> Accepted { message_id }. It is not a child UserMessageChunk and does not imply immediate sampling projection while a tool is active.
- Live projection is normal ACP `session/update`, `sessionId` = child session, `update` = `UserMessageChunk` (wire `sessionUpdate: user_message_chunk`) with text equal to admitted text.
- Chunk `_meta.modelId` remains the child's current model.
- Notification `_meta.eventId` is unique PER projected message and shared by live notification and persisted record. `_meta.agentTimestampMs` comes from build_notification_meta.
- Notification `_meta.promptId` is the CURRENT RUNNING CHILD TURN's prompt ID. It is NOT the sender caller's prompt ID and NOT `parent-message-{message_id}`. Two Steers in one child turn share promptId but have distinct eventId.
- Receipt prompt ID remains `parent-message-{message_id}`. The receipt is settled once when the owning child turn settles. Receipt identity is not a new active child turn when a Steer projects.
- Active Steer has a protected lifecycle slot, not an ordinary queued input row. A missing queue row is intentional. At the next safe point it produces child timeline/history and model input once. Queue/idle/fallback messages retain the existing protected queued-prompt lifecycle.
- Projection persists once via PersistenceMsg::Update(SessionUpdate::Acp(...)) into child's updates.jsonl and adds one ConversationItem::agent_message to child chat state. Live forwarding deliberately does not depend on resident-human local-echo suppression: sender has no local child echo.

## Authored regression

The real coordinator -> run_shell_child -> ShellChildRuntime -> SessionActor -> mock Responses sampler integration now checks two ordered Steers before completion, both gateway UserMessageChunks addressed to child, unique eventIds and shared running promptId, and one durable updates.jsonl occurrence of each marker/eventId. The active-tool variant gates a real run_terminal_command on a loopback socket; messages are accepted while that tool is blocked, no new sample is observed before tool release, and the next request includes tool result then both messages. A separately held next-sample terminal proves parent completion has not happened yet.

## Active versus authored

No build/test/check was run in this continuation. Gateway fix and tests are working-tree authoring, not evidence of runtime deployment. Parent reports PATH CLI `grok1.0.45 (22fd28fbacea) [alpha]`; that observation neither identifies an already-running leader nor proves it contains this uncommitted fix.

Pager owner can test ordinary child UserMessageChunk handling with the above running-turn promptId, two distinct eventIds, hidden child navigation, and subsequent replay dedup. No new protocol variant is needed. No claim is made that Pager's ended/superseded filtering itself caused this confirmed missing-gateway defect.
