# Agent Dashboard

The Agent Dashboard lists sessions grouped by state — by default every
top-level session in this pager process, local sessions and forks. `Ctrl+A`
switches lanes: the **subagents** lane lists the other running sessions
(subagent children spawned by any session the leader hosts), and the **all**
lane combines both sides in one list, deduplicated by session. From one
screen you can peek, reply, attach, pin, rename, stop, or dispatch a new
agent.

Not the agents modal (`/config-agents` / `/agents` — definitions and
personas), the session picker (`/resume` / `Ctrl+R`, past conversations on
disk), or the workflows run UI (`/workflow runs`).

---

## Opening the dashboard

- **`grok dashboard`** — launch the TUI into the dashboard.
- **`/dashboard`** (aliases **`/agents-dashboard`**, **`/sessions`**) — open
  from inside a session.
- **`Ctrl+\`** — same view as the slash command.

Hidden in minimal mode. Set `GROK_AGENT_DASHBOARD=0` or
`[dashboard].enabled = false` to disable.

---

## What you see

```
  main ~/xai [Choose Ctrl+l]                    ◆ 2 awaiting │ ⋮ 1 working │ ◇ 1 idle

  + New Agent                                Open Previous /resume │ Worktree Ctrl+w

▌● reviewer · audit token flow    Awaiting your input            2m
 ● implementer · fix login bug    Running: cargo test           12m
 ⋅ refactor · feat/login          Responding…                   24m
 ○ housekeeping                   idle                           1h
 ● implementer · add login tests  8 tools · 1.2k tok            14m
╭─────────────────────────────────────────────────────────────────╮
│ ❯ Dispatch a new agent                                          │
╰─ dispatch ──────────────────────────────────────────────────────╯
 ↑/↓ select (peek) · Enter open · Ctrl+R rename · Ctrl+T pin · Ctrl+X stop · ? help · Esc new
```

The **header** shows where a new agent will run — the git branch and the
working directory — with state-count chips on the right (the same glyph
and colour as the rows, plus a label). While a lane or the directory scope
deviates from the default, a `◆` chip joins the counts: `◆ subagents` /
`◆ all` for the lane, `◆ cwd` / `◆ worktree` for the scope. Click the
location (or press `Ctrl+L`) to **choose** another directory; `/cd <path>`
does the same from the dispatch box.

The **actions row** below it holds `+ New Agent` (the default cursor target
when no row is selected) and, on the right, `Open Previous` (the session
picker; workspace dashboard only) and the **worktree toggle**. `→` / `←`
move the cursor along the row in that order, stopping at either end. Like
every arrow on the dashboard, they navigate while the list has focus (`Tab`)
or while the dispatch box is empty, and edit the draft once you've typed
something; in search mode the empty box is still the query, so they edit. In
vim mode `l` / `h` do the same but need the list focused, and stay inert in
search mode. `Enter` acts like a click on the focused item — create, open the
picker, or toggle worktree mode — and `Esc` steps back to `+ New Agent`. The
same actions are always a click or `/resume` / `Ctrl+W` away. With worktree mode on inside a git repo, the row
reads `+ New Agent in Worktree` / `Disable Worktree`, and the next dispatch
creates the agent in a fresh git worktree.

Each row is one session. Sort by state (Needs input → Working → Idle →
Inactive → Completed → Failed) so same-state rows sit together, or by working
directory (`Ctrl+G` toggles). **Inactive** is roster-only sessions owned by
other pager processes that this process has not loaded — background noise, so
the section **starts collapsed** (expand with `→` / click).

**`Ctrl+A` cycles the lane**: main sessions → subagents → all → back to
main. The **subagents** lane lists the *other* running sessions — subagent
children spawned by any session the leader hosts; the **all** lane shows
main sessions and those children together, deduplicated by session id.
Every lane shares the same sorting, view, and row actions (peek, reply
after attach, pin, rename, stop, open). A child you open from either lane
keeps its parent link, so sending to it still works after the leader
restarts or reaps the child actor; a send to a reaped child without that
link is refused with a message naming the child, its parent, and how to
resume it. The header shows a `◆ subagents` or `◆ all` chip while a
non-default lane is on; `Ctrl+A` cycles back to the main sessions.

**`Ctrl+Y` cycles the directory scope**: all (the default, no filtering) →
`cwd` → `worktree` → back to all. `cwd` keeps only rows whose working
directory is the dashboard's current directory; `worktree` selects resolved
members of the current directory's canonical git worktree family — the main
checkout, its subdirectories, and every linked worktree of the same repo (the first sight
of a directory schedules cached off-thread discovery, not filesystem I/O in
render). Pending, failed and non-git lookups do not match, even at the same
cwd. The scope applies to every lane, composes with the `Ctrl+/` search filter, and lives
in memory only — a restart returns to the unscoped listing. The header
shows a `◆ cwd` / `◆ worktree` chip while a scope is on.

Pinned rows currently keep their manual order in both grouping modes. Whether
this overrides automatic running/recency ordering is pending a user decision.
Agent activity and recent updates do not move them; `Shift+↑` / `Shift+↓` requests
manual reorder. In the v2 All lane, child reorder is stored but is not yet
reflected in the combined list; that sort-input repair remains pending the same decision.

To keep **Idle** scannable, only the most recent idle agents stay visible —
the 8 freshest, plus any active within the last hour. The rest fold into a
**"N more"** row at the bottom of the group; select it and press `Enter` /
`→` (or click) to expand, `←` to re-fold. The Idle header always shows the
true total. Folding is suspended while a filter or search is active.

State icons match other session lists in Grok Build:

- `⋅`/`:`/`⸬`/`⁙` — animated spinner for **Working**
- `●` — filled circle for **Needs input**, **Completed**, **Failed**,
  **Blocked** (color: yellow / green / red / amber)
- `○` — hollow circle for **Idle** and **Inactive**

A row stays **Working** while it has live background work even if its turn
has finished — a background task, a `monitor`, or an active scheduled
`/loop`. The activity line says what is still running (for example
`1 monitor · 2 loops still running`).

State grouping displays inline section headers; directory grouping keeps the
per-row state dot and color.

**Dashboard preview** is under **Appearance** in `/settings` (search for "preview").
The list gains space when you turn off the preview and reply panel.
You must open a session to reply or answer permissions with the preview off.
The new-agent prompt stays available.
The setting persists across restarts.
You can turn it back on in the same place.
`[ui] dashboard_preview = true` in `config.toml` also enables it.

The dispatch input uses the same prompt chrome as the agent view. Press
`Ctrl+/` to flip it into **search mode**: the `❯` prefix becomes a yellow
`Search:` and typing live-filters the list instead of dispatching.

---

## Keybindings

| Key | Action |
| --- | --- |
| `↑` / `↓`, `j` / `k` | Navigate rows and section titles (selecting a row opens peek) |
| `PageUp` / `PageDown` | Move the row cursor one page up / down (works in every lane and over a confirmed filter) |
| `Home` / `End` | Jump to the first / last visible focusable, including section titles |
| `→` / `←` (on a section title) | Expand / collapse the section (`l` / `h` in vim mode) |
| `Enter` (on a section title) | Toggle the section collapsed / expanded |
| `Enter` (empty reply) | Open the selected agent full-screen (details view) |
| `Ctrl+S` | Send the peek reply and open the agent (or dispatch and attach a new session) |
| `Shift+Enter` / `Alt+Enter` | Newline in the reply / dispatch input |
| `1`–`9` | Answer a pending permission / ask question when peek shows options |
| `Enter` (typed reply) | Send / queue the reply to the selected agent |
| `/` | Literal `/` into the prompt |
| `Ctrl+/` | Toggle search mode (live-filter rows) |
| `Ctrl+A` | Cycle the lane: main sessions → subagents → all → main |
| `Ctrl+Y` | Cycle the directory scope: all → cwd → worktree → all |
| `Ctrl+R` | Rename selected row |
| `Ctrl+T` | Pin / unpin |
| `Ctrl+G` | Toggle grouping (state ↔ directory) |
| `Ctrl+X` | Cancel a running turn, or press twice within 2s to permanently delete |
| Hover + click `[✗]` | Permanently delete an idle/done row (click again to confirm) |
| `Shift+↑` / `Shift+↓` | Reorder pinned rows |
| `Esc` | Step back: cancel search → close peek → clear filter → unfocus dispatch → unselect row → exit. Never clears a typed dispatch draft (`Ctrl+U` / `Ctrl+C` for that) |
| `Ctrl+\` | Return from details view, or exit dashboard |
| `Ctrl+.` (alt: `?`) | Keyboard shortcuts cheatsheet. Footer shows `?` when `Ctrl+.` cannot be delivered. Bare `?` opens help when list-focused or the draft is empty |

When grouping by state, each group has a **section title** (for example
`Working`, `Idle`) with a `▸`/`▾` marker. Select a title and press `→` /
`←` to expand or collapse (`l` / `h` in vim mode). Click toggles; hover
brightens. Collapse state is remembered while the dashboard stays open.
**Inactive** starts collapsed each time the pager starts; expanding it sticks
until you quit.

`PageUp` / `PageDown` / `Home` / `End` page the row list under the same
ownership as `↑` / `↓`: an unfocused saved reply does not block list navigation.
A focused reply keeps the keys for its caret whenever it contains any text,
including spaces or newlines; an empty reply permits list navigation. Search
and open slash or `@` dropdowns retain their keys. Home and End include visible
section titles, not only session rows.

Opening a row shows the agent's conversation in the **details view**. The
session's own header row does the work — no extra title band: the agent's
name leads it (`name │ main ~/xai`; omitted for an unnamed session), and on
the right, after the usual chips, sit `‹ 2/5 ›` (your position among the
dashboard's agents; hidden when there is only one) and `[Dashboard]`. Keys
go to the attached agent; `Esc` / `Ctrl+\` (or `[Dashboard]`) return to the
dashboard; `‹` / `›` cycle agents. The shortcuts bar shows
`Ctrl+\: back to dashboard`. Gotcha: `Esc` only returns; `/exit` inside the
agent closes the session (dashboard toast: "Session closed").

Three more header buttons work with the mouse. `[✗]` (on a top-level session)
removes it from the `‹ i/n ›` list: the counter shrinks and the next session
opens, or the dashboard when it was the only one. The session keeps running and
stays in the dashboard; opening it again puts it back in the list, and a
restart resets the list. `[↓]` opens the session's latest subagent and `[↑]` (shown inside a
subagent) returns to its parent; they match `Ctrl+Alt+Down` / `Ctrl+Alt+Up`.

Every session with the dashboard enabled shows `[Dashboard]` in its header,
not just ones opened from the dashboard; clicking it is the same as `Ctrl+\`.
Disabling the dashboard (see the top of this page) removes the button too.

`Ctrl+X` in the details view is state-dependent. While a **turn is running**
it cancels the turn (same as `Ctrl+C`, including the keep-subagents prompt)
and never closes the session. Otherwise — **idle**, a slash command in
flight, or a cancel still pending — `Ctrl+X` arms a confirmation: press again
within 2 seconds to close the session and return to the dashboard. Any other
key cancels the confirmation; a turn that starts inside the window turns the
confirmed press into a cancel instead. (If `Ctrl+X` is also the cheatsheet
binding on your terminal, use `Ctrl+.` inside the details view.)

See [Keyboard Shortcuts](03-keyboard-shortcuts.md#agent-dashboard).

---

## Completing or closing a session

There is **no** “mark completed” command. Row state is derived from the agent:

- **Completed** / **Failed** when work ends on its own (turn finished and no
  background task / monitor / `/loop` still running).
- **`Ctrl+X` once** while a turn is running cancels the turn.
- **`Ctrl+X` twice** (within 2s) **permanently deletes** the session
  (same as `/delete`). Hover an idle/done row to swap age for `[✗]` and
  click twice to confirm.
- In the details view, `/exit` also closes the session (Esc only returns).
  `/delete` inside an attached agent wipes that session and returns to the
  dashboard.

There is no manual complete flag. Use `/exit` to leave a session without
deleting history.

---

## Dispatch input

The bottom textarea **always spawns a new session**. A selected row is the
navigation cursor, not a reply target — open an agent to talk to it.

- Free text → new top-level session seeded with the prompt. Text is never
  treated as a filter (even if it starts with `/`, `s:`, `a:`, or `#`);
  filtering is `Ctrl+/` search mode. A leading `/` runs a pager-global slash
  command.
- Empty input → open the selected row, or create a new agent when
  `+ New Agent` is focused.

`/usage` opens the usage modal over the dashboard. The dashboard has no
session, so the **Usage limit** tab shows your account allowance while the
two session tabs read "No active session"; open an agent for its context and
token totals (`/context` and `/session-info` only work inside a session).
`Esc` or `[✗]` closes it.

`Ctrl+S` after typing dispatches **and** attaches; plain `Enter` stays on the
dashboard so you can dispatch several sessions. `Shift+Enter` / `Alt+Enter`
insert a newline; the box grows with the draft (up to a cap, then scrolls).

Empty or whitespace-only prompts are ignored. Prompts above 64 KiB are
rejected with a toast.

### Focus: input bar ↔ overview list (`Tab`)

Two focus areas: the **dispatch input** and the **overview list**. `Tab`
toggles between them; the inactive input dims its border and hides its caret.

On open, focus defaults to the **overview list** when at least one agent
exists (so `↑`/`↓` / vim `j`/`k` navigate immediately). With **no** agents,
focus stays on the **dispatch input**. Either way, the cursor starts on
`+ New Agent` (no agent row pre-selected).

- **Input focused**: type a new-session prompt. Empty prompt: `↑`/`↓`
  navigate rows; non-empty: move the caret. `Esc` unfocuses to the list
  (draft kept).
- **Overview focused**: `↑`/`↓` (and vim `j`/`k`) move between rows. `Enter`
  opens the highlighted agent (on `+ New Agent`, sends a typed draft or
  creates a new session). `Esc` stays on the list and steps back — clear
  filter, then unselect (→ `+ New Agent`), then exit. `Tab`, `i` (vim), or
  any printable key returns to the input.

---

## Peek panel

Selecting an agent row shows the **peek panel** in place of the dispatch box.
With no row selected (`+ New Agent`, or after `Esc`), the dispatch box
returns. Select a row to talk to an existing agent; deselect to start a new
one.

Top to bottom: header (**last response type** — `Thinking` / `Thought` /
`Response` / `Edit` / `Read` / `Bash` / … — and **time**), the most recent
response (word-wrapped, up to ~3 rows; `…` when truncated), and a live
`❯ reply` input.

The selected agent's **model** and, in always-approve (yolo) mode, an
**`always-approve`** flag sit on the panel's bottom border (same badge slot as
the dispatch box), including while answering questions. List rows no longer
repeat model or always-approve badges.

**`Shift+Tab` cycles the peeked agent's mode** (Normal → Plan → Auto-review
(when enabled) → Always-approve → Normal) on the **live** agent. On the dispatch box,
Shift+Tab only stages mode for the *next* agent.

Unlike dispatch (new sessions only), peek reply **talks to the selected
agent**:

- **Type into `❯ reply`, then `Enter`** to send. Idle agents start immediately;
  busy agents **queue** the message (same as the agent view prompt). `Ctrl+S`
  replies and opens the detail view; `Shift+Enter` / `Alt+Enter` insert a
  newline (reply grows with the draft).
- Empty reply + `Enter` opens the agent.
- **`↑`/`↓` move the caret** once the reply has content. While empty (or
  unfocused via `Tab`), `↑`/`↓` **switch the selected agent** — the panel
  follows, and a half-typed draft is cleared so it cannot land on the wrong
  agent. (`Tab` to the list to navigate while a draft is in the reply.)
- **`Esc` unselects**: clear a typed reply first, then deselect and focus
  `+ New Agent`.
- **`Tab`** toggles focus between reply and row list; a printable key
  re-focuses the reply.
- Full prompt editor (same as dispatch / agent prompt): multi-line paste
  chips, mouse select, word navigation, `Ctrl+A`/`Ctrl+E`, `Alt+Backspace`,
  `Ctrl+W`/`Ctrl+U`/`Ctrl+K`, undo, Shift+arrow selection, `Ctrl+Shift+V`
  inline paste. **`@`** opens the file picker rooted at the **peeked agent's**
  working directory; the dropdown floats above the panel. Dashboard chords
  (`Ctrl+X` stop, `Ctrl+T` pin, `Shift+↑/↓` reorder, …) still win while the
  panel is open.
- Pending **permission / ask-tool** question: `❯ reply` hides; options list
  instead. **`↑`/`↓` highlight**, **`Enter` answers**, **`1`–`9`** answer
  directly. Free-text **No / reject** and ask-tool **Other** accept a typed
  answer on the free-text row. Multi-question Ask forms walk one at a time
  (`(i/N)`); multi-select forms need the agent's own view.

On very short terminals the panel may not fit; the dispatch box stays even
with a row selected.

---

## Search / filter (`Ctrl+/`)

`Ctrl+/` toggles search mode so normal typing always dispatches. Prefix
flips from `❯` to yellow `Search:`; every keystroke live-filters the list.
The text filter composes with the `Ctrl+Y` directory scope: the scope picks
the directory family, the query narrows inside it.

- `Enter` — confirm: keep the filter and return to the dispatch prompt.
- `Esc` or `Ctrl+/` — cancel: clear the filter and exit search.
- `↑` / `↓` — navigate filtered rows; `PageUp` / `PageDown` / `Home` / `End`
  page through them.

Prefixes (only inside search mode):

- `a:<name>` — agent label (case-insensitive substring; persona / role).
- `s:<state>` — row state: `working`, `idle`, `completed`, `failed`,
  `needs-input`, `blocked` and synonyms (`busy`/`running`/`done`/etc.).
- `#<text>` — substring match on `#<text>` (literal `#` in labels).
- anything else — substring over label + working dir.

---

## Persistence

Per-user preferences under `[dashboard]` in `~/.grok/config.toml`:

```toml
[dashboard]
enabled = true
grouping = "state"   # or "directory"
pinned   = ["top:<session_id>"]
reorder  = ["top:<session_id>"]
```

Pinned/reorder entries use **session id** (not a per-process agent slot), so
they survive restarts. Pins follow the row's role: a pin or reorder made on
a subagent-lane row (or a child row in the all lane) is written to the
scoped child layout, so it never reorders your main sessions; in the
workspace dashboard, top-level pins live in the workspace store. The lane
and the directory scope are in-memory only — a restart opens the main lane,
unscoped.
