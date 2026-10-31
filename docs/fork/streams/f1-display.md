# F1-display — message presentation

## Message display

F7 Message display controls presentation per message kind. Its first group is
**Instructions / capabilities**, using the existing `InjectedInstructions` kind.
Conversation follows it. Instruction settings have one owner; prior
`injected/injected_instructions` preferences remain readable and inherited.

Ordinary activation cycles `1 line → full → hide → 1 line`; Backspace reverses
the cycle. Explicit expand/collapse and existing bulk folding remain two-state
gestures. They preserve an explicit Hidden choice. Existing effective defaults
remain unchanged; Hide requires an explicit selection. Global and session panel
settings persist the `hidden` value through the existing settings layers.

`DisplayForm` contains `Collapsed`, `Expanded` and `Hidden`. `DisplayMode` also
contains `Truncated`, the existing transient renderer state. `DisplayDefaults`
holds explicit choices; `EntryForm` retains kind-following and entry-override
semantics. `ScrollbackState::apply_kind_default` owns their precedence.

## Visibility and retained content

Collapsed output uses the existing one-row renderer policy. Expanded output uses
the existing complete body renderer. Hidden output has zero rows, spacers,
selection ranges and hit regions. The shared visibility policy also applies to
sticky prompts, navigation, inline sources, Peek and native scrollback output.
The underlying body, history, export source and provider-request inputs remain
available. Restoring a visible form reuses the retained entry and content.

Search reveal expands group truncation before restoring selection and settling
the selected Turn's geometry. Explicit Hide remains a display choice; search
reveal leaves it intact. Hidden inline children create no passive replay or body
projection demand, including demands retained from a previous frame. Visible
inline bodies share the entry owner's `Truncated | Expanded` classification.
Execute-to-background demotion uses the canonical block replacement and kind
default owner, retaining stdout, description, identity and original time data.

Native commit and reprint paths preserve Hidden. Terminal output already printed
before the setting changed remains in the terminal's existing scrollback.

## Existing renderer integration

The current renderers retain native glyphs, timestamps, full source text and
selection joiners. Thinking sections preserve stream paragraph boundaries and
chronological placement around tool rows. Background stdout and child transcript
bodies use their existing canonical source owners; passive child reads remain
separate from execution or resume operations. Explicit copy uses those retained
sources and originating identities.

Image-only transcript entries use the existing OS-native opener independently
of terminal graphics support. Text containing image references retains the text
viewer. Inline playback and prompt-chip previews retain their graphics
capability checks.

The instruction-source projection is a separate integration: this display
package supplies its existing category, renderer and visibility controls. It
does not establish that every actual provider instruction is already published
into the transcript.

## Verification and remaining evidence

Regression sources cover defaults and legacy serialization, every-kind cycling,
group order and inheritance, global/session reopening, retained captured text,
ACP silence, zero-row geometry and selection, restore and finish/fold behavior.
Additional real-path oracles cover grouped search reveal, explicit Hide across
Turns, previously rendered children with stale replay demands, and background
demotion with a Hidden kind preference. Existing assertions remain intact.

Two independent source reviews covered the package and its three corrected
regressions. Compilation, tests, visual acceptance and actual provider-request
byte equivalence remain unverified. Verification runs through GitLab. Separate
renderer resource and viewport-lazy cold-projection work remains tracked outside
this package. Design context: `docs/fork/design/f6-f7.md`.
