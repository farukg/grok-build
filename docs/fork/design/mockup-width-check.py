"""Generate screen-width contracts for the styled F6/F7 mockups."""
from pathlib import Path
import unicodedata

ROOT = Path(__file__).resolve().parent
OUT = ROOT / 'f6-f7-mockups.md'

def width(text):
    return sum(0 if unicodedata.combining(c) else 2 if unicodedata.east_asian_width(c) in 'WF' else 1 for c in text)

def fit(text, cells):
    if width(text) > cells:
        remaining = cells - 1
        result = ''
        for c in text:
            w = width(c)
            if w > remaining:
                break
            result += c
            remaining -= w
        text = result + '…'
    return text + ' ' * (cells - width(text))

CATEGORIES = [('Core instructions','OFF','2.1k'),('Project rules','ON','6.4k'),('Environment','ON','0.3k'),('Skills / workflows','ON','1.5k'),('MCP catalog','OFF','0.8k'),('Tool definitions','ON','4.0k'),('User history','ON','8.2k'),('Assistant','ON','4.1k'),('Reasoning','OFF','3.3k'),('Tool exchanges','ON','1.7k'),('Runtime notices','ON','0.5k'),('Memory','OFF','0.7k'),('Summary','ON','1.2k')]
KINDS = [('User prompt','1 line'),('Assistant','full'),('Execute','1 line'),('Read','1 line'),('Edit','1 line'),('ListDir','1 line'),('Search','1 line'),('WebFetch','1 line'),('WebSearch','1 line'),('Integration search','1 line'),('UseTool','1 line'),('MemorySearch','1 line'),('SentMessage','1 line'),('Skill','1 line'),('Other tool','1 line'),('Thinking','1 line'),('System','full'),('Session event','1 line'),('Background task','1 line'),('Subagent','1 line'),('Workflow','full'),('BTW','full'),('Context info','1 line'),('Memory capture','1 line')]
# Eighteen sidebar viewport rows: remaining kinds are reachable by scrolling the same list.
DOCK = ['▾ Dock', '  ▾ Subagents 3', '  ◆ Verify cycle fix builds ◌ ↻ 5m40s 135K/872K [↗][X]', '  ◆ Resume M11 after restart ◌ ↻ 5m40s 135K/872K [↗][X]', '  ◆ Resume M5 after restart ◌ ↻ 5m40s 135K/872K [↗][X]', '']

def compact_dock(text, n):
    if width(text) <= n:return fit(text,n)
    prefix, actions = text.rsplit(' [↗][X]',1)
    head, meta = prefix.split(' ◌ ',1)
    fixed = ' ◌ ' + meta + ' [↗][X]'
    return fit(head,max(0,n-width(fixed))) + fixed

def sidebar(n):
    left = (n-1)//2
    right = n-left-1
    rows = [compact_dock(x,n) if '[↗][X]' in x else fit(x,n) for x in DOCK]
    rows.append(fit('▾ Context',left)+'│'+fit('▾ Message Display',right))
    for i,(label,on,tok) in enumerate(CATEGORIES):
        form = f'{KINDS[i][0]} {KINDS[i][1]}'
        # Small sidebars expose a scrollable two-column viewport; no category is silently omitted.
        l = fit(label,max(0,left-8))+fit(on,3)+fit('~'+tok,5)
        rows.append(l+'│'+fit(form,right))
    rows += [fit('  Current request · always sent', n),fit('  ↑↓ scroll categories / kinds', n)]
    return rows

OUTLINE = ['▾ Turn 19  [Image #1] ⇒ ich will…','  ▾ Read 3 files','    ◆ CLAUDE.md','    ◆ timeline_panel.rs','    ◆ timeline.rs','  ▸ Searched 2 patterns','  ▸ Edited 1 file','  Antwort · thinking 7.1s · 38s','','▸ Turn 20  Resume M11 after restart','▸ Turn 21  Resume M5 after restart']
CHAT = ['',' ❯ [Image #1] ich will dass dieser obere Bereich in die F7-sidebar kommt','  16:14  ·  1 image','',' ◆ Read CLAUDE.md',' ◆ Read timeline_panel.rs',' ◆ Read timeline.rs',' ◆ Searched 2 patterns',' ◆ Edited 1 file',' ◆ Thinking for 7.1s','','The timeline can show the actions alongside each message,','and the sidebar can expose exactly what reaches the model.','','Worked for 38s','',' ❯ Resume M11 after restart','  16:16','',' ◆ Run cargo test -p xai-grok-pager',' ◆ Thinking for 5.4s','','Worked for 12s']
SPECS = [
 ('180 columns · F6 + F7',180,38,77,60,'both180'),
 ('140 columns · F6 + F7',140,32,47,56,'both140'),
 ('120 columns · both open; F6 active overlay',120,34,82,0,'overlay'),
 ('120 columns · F6 only',120,34,82,0,'f6'),
 ('120 columns · F7 only',120,2,53,60,'f7'),
 ('120 columns · rail only',120,2,114,0,'rail')]

def screen(spec, hover=False):
    title, total, left, middle, right, mode = spec
    assert left+middle+right+(5 if right else 4)==total
    side = sidebar(right) if right else []
    result = []
    for y in range(32):
        if y == 0:
            line = fit(' main  │  ~/projects/org/forks/grok-build  ◆ 3  [Goal: TUI parity]  135K tokens  5m40s  │  135K/872K  │  ‹ 1/3 ›  │  [Dashboard]',total)
        elif y >= 27:
            full = {27:' ╭'+'─'*(total-4)+'╮',28:' │  ' + fit('Write a message…',total-7)+'│',29:' │  ' + fit('sigma/role/general  ·  grok-4.6  ·  yolo',total-7)+'│',30:' ╰'+'─'*(total-4)+'╯',31:' Ctrl+\\:dashboard  │  Ctrl+[/]:prev/next agent  │  F6:timeline  │  F7:context  │  ?:help'}[y]
            line = fit(full,total)
        else:
            pos = y-1
            rail = '  '
            if pos == 0: rail = '▴ '
            if pos in (1,5): rail = ' ─'
            if pos in (2,3,4): rail = ' ━'
            if pos == 10: rail = '▾ '
            if left>2:
                text = OUTLINE[pos] if pos<len(OUTLINE) else ''
                if pos==2: text='▏ '+text.strip()
                if pos==5: text='· '+text.strip()
                a=fit(rail,2)+fit(text,left-2)
            else:a=fit(rail,left)
            content=CHAT[pos] if pos<len(CHAT) else ''
            if pos==1: content=' '+fit(content.strip(),middle-2)+' '
            if hover and pos in (16,17,18):
                content={16:'╭ F6 detail  ·  Searched 2 patterns',17:'│ /home/fg/projects/org/forks/grok-build/crates/codegen/',18:'╰ xai-grok-pager/src/views/timeline_panel.rs'}[pos]
            b=fit(content,middle)
            if right:
                c=side[pos] if pos<len(side) else fit('',right)
                if hover and pos in (23,24,25):
                    c=fit({23:'╭ Resume M11 after restart',24:'│ ↻ sigma/role/general · 5m40s',25:'╰ ◌ Waiting · 135K/872K'}[pos],right)
                line='│'+a+'│'+b+'▏│'+c+'│'
            else:line='│'+a+'│'+b+'▏│'
        assert width(line)==total,(title,y,width(line),total,line)
        if 1<=y<=26:
            # The interior Context/Display divider is a separate F7 internal column.
            bounds=[0,left+1,left+middle+3]
            if right:bounds.append(total-1)
            assert all(line[p]=='│' for p in bounds),(title,y,bounds,line)
        result.append(line)
    return result

def main():
    parts=['# F6/F7 colored screen mockups and cell-width contracts','',
           'Theme: Faruk’s Windows Terminal defaults to Color DNA; Grok `color_dna()` RGB values are used in the PNGs. Font: the Windows Terminal default `Hack NF FC Ligatured CCG` bold. The full header, bordered composer and shortcut hint are outside the transcript/sidebar columns, as in the reference screenshot. These are proposed-state renderings, not captures of an implemented UI.','',
           'The three F7 sections are Dock, Context, Message Display. Only nonempty dock section Subagents appears. Context and Message Display use two scrollable columns under distinct headers to fit the available terminal height; all 13 categories are shown, with remaining message kinds accessible by scrolling. `◆` = `diamond_filled`; `▸`/`▾` = disclosure glyphs; `▴`/`▾` in rail = timeline chevrons; `━` = `heavy_horizontal` (one-cell, dim viewport range); `─` = `light_horizontal`; `▏` = selection bar in F6 or chat scrollbar at its right edge; `◌` = state glyph; `↻` = resumed glyph; `[↗][X]` = unchanged action buttons. Legacy-console fallbacks come from `crate::glyphs`.','',
           'Color roles: `theme.bg_base` continuous background, `theme.bg_light` raised user card, `theme.text_primary` prose, `theme.text_secondary` metadata, `theme.gray_dim` separators/sizes/idle rail, `theme.gray_bright` section labels, `theme.accent_tool` tool glyphs, `theme.accent_thinking` thinking, `theme.accent_running` subtle viewport accent, `theme.selection_overlay()` selected F6 row, `theme.hover_overlay()` hovered row, `theme.prompt_border` composer. Clipped descriptions/model are fully available in the hover/selection variant; activity/resume state, elapsed and tokens/window remain in the row.','']
    reports=[]
    for spec in SPECS:
        title,n,*_=spec
        rows=screen(spec)
        parts.extend(['## '+title,'','```text',*rows,'```',''])
        reports.append(f'{title}: PASS 32/32 rows = {n} cells; borders aligned')
    parts.extend(['## PNG renderings','',*[f'- [`{name}`](mockups/{name})' for name in ('180-both.png','140-both.png','120-overlay.png','120-f6.png','120-f7.png','120-rail.png','180-hover.png')],'','## Width-check output','','```text',*reports,'```','','Checker: `mockup-width-check.py` calculates Unicode display cells with `unicodedata.east_asian_width` (W/F = 2, combining = 0). Every row and pane-boundary assertion is mandatory; any off-by-one aborts rendering.',''])
    OUT.write_text('\n'.join(parts))
    print('\n'.join(reports))

if __name__=='__main__':main()
