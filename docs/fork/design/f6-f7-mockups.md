# F6/F7 colored screen mockups and cell-width contracts

Theme: Faruk’s Windows Terminal defaults to Color DNA; Grok `color_dna()` RGB values are used in the PNGs. Font: the Windows Terminal default `Hack NF FC Ligatured CCG` bold. The full header, bordered composer and shortcut hint are outside the transcript/sidebar columns, as in the reference screenshot. These are proposed-state renderings, not captures of an implemented UI.

The three F7 sections are Dock, Context, Message Display. Only nonempty dock section Subagents appears. Context and Message Display use two scrollable columns under distinct headers to fit the available terminal height; all 13 categories are shown, with remaining message kinds accessible by scrolling. `◆` = `diamond_filled`; `▸`/`▾` = disclosure glyphs; `▴`/`▾` in rail = timeline chevrons; `━` = `heavy_horizontal` (one-cell, dim viewport range); `─` = `light_horizontal`; `▏` = selection bar in F6 or chat scrollbar at its right edge; `◌` = state glyph; `↻` = resumed glyph; `[↗][X]` = unchanged action buttons. Legacy-console fallbacks come from `crate::glyphs`.

Color roles: `theme.bg_base` continuous background, `theme.bg_light` raised user card, `theme.text_primary` prose, `theme.text_secondary` metadata, `theme.gray_dim` separators/sizes/idle rail, `theme.gray_bright` section labels, `theme.accent_tool` tool glyphs, `theme.accent_thinking` thinking, `theme.accent_running` subtle viewport accent, `theme.selection_overlay()` selected F6 row, `theme.hover_overlay()` hovered row, `theme.prompt_border` composer. Clipped descriptions/model are fully available in the hover/selection variant; activity/resume state, elapsed and tokens/window remain in the row.

## 180 columns · F6 + F7

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [Dashboard]                                                    
│▴ ▾ Turn 19  [Image #1] ⇒ ich will…   │                                                                             ▏│▾ Dock                                                      │
│ ─  ▾ Read 3 files                    │ ❯ [Image #1] ich will dass dieser obere Bereich in die F7-sidebar kommt     ▏│  ▾ Subagents 3                                             │
│ ━▏ ◆ CLAUDE.md                       │  16:14  ·  1 image                                                          ▏│  ◆ Verify cycle fix builds ◌ ↻ 5m40s 135K/872K [↗][X]      │
│ ━    ◆ timeline_panel.rs             │                                                                             ▏│  ◆ Resume M11 after restart ◌ ↻ 5m40s 135K/872K [↗][X]     │
│ ━    ◆ timeline.rs                   │ ◆ Read CLAUDE.md                                                            ▏│  ◆ Resume M5 after restart ◌ ↻ 5m40s 135K/872K [↗][X]      │
│ ─· ▸ Searched 2 patterns             │ ◆ Read timeline_panel.rs                                                    ▏│                                                            │
│    ▸ Edited 1 file                   │ ◆ Read timeline.rs                                                          ▏│▾ Context                    │▾ Message Display             │
│    Antwort · thinking 7.1s · 38s     │ ◆ Searched 2 patterns                                                       ▏│Core instructions    OFF~2.1k│User prompt 1 line            │
│                                      │ ◆ Edited 1 file                                                             ▏│Project rules        ON ~6.4k│Assistant full                │
│  ▸ Turn 20  Resume M11 after restart │ ◆ Thinking for 7.1s                                                         ▏│Environment          ON ~0.3k│Execute 1 line                │
│▾ ▸ Turn 21  Resume M5 after restart  │                                                                             ▏│Skills / workflows   ON ~1.5k│Read 1 line                   │
│                                      │The timeline can show the actions alongside each message,                    ▏│MCP catalog          OFF~0.8k│Edit 1 line                   │
│                                      │and the sidebar can expose exactly what reaches the model.                   ▏│Tool definitions     ON ~4.0k│ListDir 1 line                │
│                                      │                                                                             ▏│User history         ON ~8.2k│Search 1 line                 │
│                                      │Worked for 38s                                                               ▏│Assistant            ON ~4.1k│WebFetch 1 line               │
│                                      │                                                                             ▏│Reasoning            OFF~3.3k│WebSearch 1 line              │
│                                      │ ❯ Resume M11 after restart                                                  ▏│Tool exchanges       ON ~1.7k│Integration search 1 line     │
│                                      │  16:16                                                                      ▏│Runtime notices      ON ~0.5k│UseTool 1 line                │
│                                      │                                                                             ▏│Memory               OFF~0.7k│MemorySearch 1 line           │
│                                      │ ◆ Run cargo test -p xai-grok-pager                                          ▏│Summary              ON ~1.2k│SentMessage 1 line            │
│                                      │ ◆ Thinking for 5.4s                                                         ▏│  Current request · always sent                             │
│                                      │                                                                             ▏│  ↑↓ scroll categories / kinds                              │
│                                      │Worked for 12s                                                               ▏│                                                            │
│                                      │                                                                             ▏│                                                            │
│                                      │                                                                             ▏│                                                            │
│                                      │                                                                             ▏│                                                            │
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                                                                             │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                                                                                     │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                                                                            
```

## 140 columns · F6 + F7

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [Dashboard]            
│▴ ▾ Turn 19  [Image #1] ⇒ ich w…│                                               ▏│▾ Dock                                                  │
│ ─  ▾ Read 3 files              │ ❯ [Image #1] ich will dass dieser obere Bere… ▏│  ▾ Subagents 3                                         │
│ ━▏ ◆ CLAUDE.md                 │  16:14  ·  1 image                            ▏│  ◆ Verify cycle fix builds ◌ ↻ 5m40s 135K/872K [↗][X]  │
│ ━    ◆ timeline_panel.rs       │                                               ▏│  ◆ Resume M11 after restart ◌ ↻ 5m40s 135K/872K [↗][X] │
│ ━    ◆ timeline.rs             │ ◆ Read CLAUDE.md                              ▏│  ◆ Resume M5 after restart ◌ ↻ 5m40s 135K/872K [↗][X]  │
│ ─· ▸ Searched 2 patterns       │ ◆ Read timeline_panel.rs                      ▏│                                                        │
│    ▸ Edited 1 file             │ ◆ Read timeline.rs                            ▏│▾ Context                  │▾ Message Display           │
│    Antwort · thinking 7.1s · 3…│ ◆ Searched 2 patterns                         ▏│Core instructions  OFF~2.1k│User prompt 1 line          │
│                                │ ◆ Edited 1 file                               ▏│Project rules      ON ~6.4k│Assistant full              │
│  ▸ Turn 20  Resume M11 after r…│ ◆ Thinking for 7.1s                           ▏│Environment        ON ~0.3k│Execute 1 line              │
│▾ ▸ Turn 21  Resume M5 after re…│                                               ▏│Skills / workflows ON ~1.5k│Read 1 line                 │
│                                │The timeline can show the actions alongside ea…▏│MCP catalog        OFF~0.8k│Edit 1 line                 │
│                                │and the sidebar can expose exactly what reache…▏│Tool definitions   ON ~4.0k│ListDir 1 line              │
│                                │                                               ▏│User history       ON ~8.2k│Search 1 line               │
│                                │Worked for 38s                                 ▏│Assistant          ON ~4.1k│WebFetch 1 line             │
│                                │                                               ▏│Reasoning          OFF~3.3k│WebSearch 1 line            │
│                                │ ❯ Resume M11 after restart                    ▏│Tool exchanges     ON ~1.7k│Integration search 1 line   │
│                                │  16:16                                        ▏│Runtime notices    ON ~0.5k│UseTool 1 line              │
│                                │                                               ▏│Memory             OFF~0.7k│MemorySearch 1 line         │
│                                │ ◆ Run cargo test -p xai-grok-pager            ▏│Summary            ON ~1.2k│SentMessage 1 line          │
│                                │ ◆ Thinking for 5.4s                           ▏│  Current request · always sent                         │
│                                │                                               ▏│  ↑↓ scroll categories / kinds                          │
│                                │Worked for 12s                                 ▏│                                                        │
│                                │                                               ▏│                                                        │
│                                │                                               ▏│                                                        │
│                                │                                               ▏│                                                        │
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                                     │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                                             │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                                    
```

## 120 columns · both open; F6 active overlay

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [D…
│▴ ▾ Turn 19  [Image #1] ⇒ ich wil…│                                                                                  ▏│
│ ─  ▾ Read 3 files                │ ❯ [Image #1] ich will dass dieser obere Bereich in die F7-sidebar kommt          ▏│
│ ━▏ ◆ CLAUDE.md                   │  16:14  ·  1 image                                                               ▏│
│ ━    ◆ timeline_panel.rs         │                                                                                  ▏│
│ ━    ◆ timeline.rs               │ ◆ Read CLAUDE.md                                                                 ▏│
│ ─· ▸ Searched 2 patterns         │ ◆ Read timeline_panel.rs                                                         ▏│
│    ▸ Edited 1 file               │ ◆ Read timeline.rs                                                               ▏│
│    Antwort · thinking 7.1s · 38s │ ◆ Searched 2 patterns                                                            ▏│
│                                  │ ◆ Edited 1 file                                                                  ▏│
│  ▸ Turn 20  Resume M11 after res…│ ◆ Thinking for 7.1s                                                              ▏│
│▾ ▸ Turn 21  Resume M5 after rest…│                                                                                  ▏│
│                                  │The timeline can show the actions alongside each message,                         ▏│
│                                  │and the sidebar can expose exactly what reaches the model.                        ▏│
│                                  │                                                                                  ▏│
│                                  │Worked for 38s                                                                    ▏│
│                                  │                                                                                  ▏│
│                                  │ ❯ Resume M11 after restart                                                       ▏│
│                                  │  16:16                                                                           ▏│
│                                  │                                                                                  ▏│
│                                  │ ◆ Run cargo test -p xai-grok-pager                                               ▏│
│                                  │ ◆ Thinking for 5.4s                                                              ▏│
│                                  │                                                                                  ▏│
│                                  │Worked for 12s                                                                    ▏│
│                                  │                                                                                  ▏│
│                                  │                                                                                  ▏│
│                                  │                                                                                  ▏│
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                 │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                         │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                
```

## 120 columns · F6 only

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [D…
│▴ ▾ Turn 19  [Image #1] ⇒ ich wil…│                                                                                  ▏│
│ ─  ▾ Read 3 files                │ ❯ [Image #1] ich will dass dieser obere Bereich in die F7-sidebar kommt          ▏│
│ ━▏ ◆ CLAUDE.md                   │  16:14  ·  1 image                                                               ▏│
│ ━    ◆ timeline_panel.rs         │                                                                                  ▏│
│ ━    ◆ timeline.rs               │ ◆ Read CLAUDE.md                                                                 ▏│
│ ─· ▸ Searched 2 patterns         │ ◆ Read timeline_panel.rs                                                         ▏│
│    ▸ Edited 1 file               │ ◆ Read timeline.rs                                                               ▏│
│    Antwort · thinking 7.1s · 38s │ ◆ Searched 2 patterns                                                            ▏│
│                                  │ ◆ Edited 1 file                                                                  ▏│
│  ▸ Turn 20  Resume M11 after res…│ ◆ Thinking for 7.1s                                                              ▏│
│▾ ▸ Turn 21  Resume M5 after rest…│                                                                                  ▏│
│                                  │The timeline can show the actions alongside each message,                         ▏│
│                                  │and the sidebar can expose exactly what reaches the model.                        ▏│
│                                  │                                                                                  ▏│
│                                  │Worked for 38s                                                                    ▏│
│                                  │                                                                                  ▏│
│                                  │ ❯ Resume M11 after restart                                                       ▏│
│                                  │  16:16                                                                           ▏│
│                                  │                                                                                  ▏│
│                                  │ ◆ Run cargo test -p xai-grok-pager                                               ▏│
│                                  │ ◆ Thinking for 5.4s                                                              ▏│
│                                  │                                                                                  ▏│
│                                  │Worked for 12s                                                                    ▏│
│                                  │                                                                                  ▏│
│                                  │                                                                                  ▏│
│                                  │                                                                                  ▏│
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                 │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                         │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                
```

## 120 columns · F7 only

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [D…
│▴ │                                                     ▏│▾ Dock                                                      │
│ ─│ ❯ [Image #1] ich will dass dieser obere Bereich in… ▏│  ▾ Subagents 3                                             │
│ ━│  16:14  ·  1 image                                  ▏│  ◆ Verify cycle fix builds ◌ ↻ 5m40s 135K/872K [↗][X]      │
│ ━│                                                     ▏│  ◆ Resume M11 after restart ◌ ↻ 5m40s 135K/872K [↗][X]     │
│ ━│ ◆ Read CLAUDE.md                                    ▏│  ◆ Resume M5 after restart ◌ ↻ 5m40s 135K/872K [↗][X]      │
│ ─│ ◆ Read timeline_panel.rs                            ▏│                                                            │
│  │ ◆ Read timeline.rs                                  ▏│▾ Context                    │▾ Message Display             │
│  │ ◆ Searched 2 patterns                               ▏│Core instructions    OFF~2.1k│User prompt 1 line            │
│  │ ◆ Edited 1 file                                     ▏│Project rules        ON ~6.4k│Assistant full                │
│  │ ◆ Thinking for 7.1s                                 ▏│Environment          ON ~0.3k│Execute 1 line                │
│▾ │                                                     ▏│Skills / workflows   ON ~1.5k│Read 1 line                   │
│  │The timeline can show the actions alongside each mes…▏│MCP catalog          OFF~0.8k│Edit 1 line                   │
│  │and the sidebar can expose exactly what reaches the …▏│Tool definitions     ON ~4.0k│ListDir 1 line                │
│  │                                                     ▏│User history         ON ~8.2k│Search 1 line                 │
│  │Worked for 38s                                       ▏│Assistant            ON ~4.1k│WebFetch 1 line               │
│  │                                                     ▏│Reasoning            OFF~3.3k│WebSearch 1 line              │
│  │ ❯ Resume M11 after restart                          ▏│Tool exchanges       ON ~1.7k│Integration search 1 line     │
│  │  16:16                                              ▏│Runtime notices      ON ~0.5k│UseTool 1 line                │
│  │                                                     ▏│Memory               OFF~0.7k│MemorySearch 1 line           │
│  │ ◆ Run cargo test -p xai-grok-pager                  ▏│Summary              ON ~1.2k│SentMessage 1 line            │
│  │ ◆ Thinking for 5.4s                                 ▏│  Current request · always sent                             │
│  │                                                     ▏│  ↑↓ scroll categories / kinds                              │
│  │Worked for 12s                                       ▏│                                                            │
│  │                                                     ▏│                                                            │
│  │                                                     ▏│                                                            │
│  │                                                     ▏│                                                            │
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                 │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                         │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                
```

## 120 columns · rail only

```text
 main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [D…
│▴ │                                                                                                                  ▏│
│ ─│ ❯ [Image #1] ich will dass dieser obere Bereich in die F7-sidebar kommt                                          ▏│
│ ━│  16:14  ·  1 image                                                                                               ▏│
│ ━│                                                                                                                  ▏│
│ ━│ ◆ Read CLAUDE.md                                                                                                 ▏│
│ ─│ ◆ Read timeline_panel.rs                                                                                         ▏│
│  │ ◆ Read timeline.rs                                                                                               ▏│
│  │ ◆ Searched 2 patterns                                                                                            ▏│
│  │ ◆ Edited 1 file                                                                                                  ▏│
│  │ ◆ Thinking for 7.1s                                                                                              ▏│
│▾ │                                                                                                                  ▏│
│  │The timeline can show the actions alongside each message,                                                         ▏│
│  │and the sidebar can expose exactly what reaches the model.                                                        ▏│
│  │                                                                                                                  ▏│
│  │Worked for 38s                                                                                                    ▏│
│  │                                                                                                                  ▏│
│  │ ❯ Resume M11 after restart                                                                                       ▏│
│  │  16:16                                                                                                           ▏│
│  │                                                                                                                  ▏│
│  │ ◆ Run cargo test -p xai-grok-pager                                                                               ▏│
│  │ ◆ Thinking for 5.4s                                                                                              ▏│
│  │                                                                                                                  ▏│
│  │Worked for 12s                                                                                                    ▏│
│  │                                                                                                                  ▏│
│  │                                                                                                                  ▏│
│  │                                                                                                                  ▏│
 ╭────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ 
 │  Write a message…                                                                                                 │  
 │  sigma/role/general  ·  grok-4.6  ·  yolo                                                                         │  
 ╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ 
 Ctrl+\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help                                
```

## PNG renderings

- [`180-both.png`](mockups/180-both.png)
- [`140-both.png`](mockups/140-both.png)
- [`120-overlay.png`](mockups/120-overlay.png)
- [`120-f6.png`](mockups/120-f6.png)
- [`120-f7.png`](mockups/120-f7.png)
- [`120-rail.png`](mockups/120-rail.png)
- [`180-hover.png`](mockups/180-hover.png)

## Width-check output

```text
180 columns · F6 + F7: PASS 32/32 rows = 180 cells; borders aligned
140 columns · F6 + F7: PASS 32/32 rows = 140 cells; borders aligned
120 columns · both open; F6 active overlay: PASS 32/32 rows = 120 cells; borders aligned
120 columns · F6 only: PASS 32/32 rows = 120 cells; borders aligned
120 columns · F7 only: PASS 32/32 rows = 120 cells; borders aligned
120 columns · rail only: PASS 32/32 rows = 120 cells; borders aligned
```

Checker: `mockup-width-check.py` calculates Unicode display cells with `unicodedata.east_asian_width` (W/F = 2, combining = 0). Every row and pane-boundary assertion is mandatory; any off-by-one aborts rendering.
