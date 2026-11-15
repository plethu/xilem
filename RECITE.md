# Recite Writer patches

This branch carries fixes that [Recite](https://github.com/plethu/recite)'s Writer needs in Masonry,
on top of Xilem revision `dc496876a2f6ff2ce7687bfc8c19bd311bb553dc`. Each fix is its own commit,
so it can be dropped on its own. None of them is upstream; current contribution policies rule out
proposing them, so each stays until equivalent upstream behaviour lands. Recite depends on this
branch by pinned revision; this file is the only change beyond those commits.

The commits on this branch add: bounded text undo/redo, an explicit accessible text
field name, placeholder font scaling on mount and live property changes, shared checkbox/radio
indicator sizing on mount and live property changes, clean exit after accessibility
actions, rejection of unhandled Control/Meta text chords, scrollbar normalization after dynamic content
resizing, and safe display-owned clipboard construction. The clipboard change requires the
[plethu/copypasta](https://github.com/plethu/copypasta/tree/integration/recite-writer) branch. The host also exposes explicit signal delivery after direct native focus changes,
so redraw and accessibility updates complete without another input event. Explicit AltGraph and
Option text remain supported by the text-input guard. These are
local corrections, not verified upstream backports. No unsafe-code lint exception is added to Masonry.
Remove these overrides when equivalent upstream behavior is verified.

Recite previously carried byte-range `Label` styles and a `Label` line cap here. Recite's own
`ShapedText` widget now provides both (plethu/recite#288), so this branch no longer carries them.

Masonry's compose pass additionally rebuilds an accessibility node only when that widget's own
transform changes. Upstream marked every descendant of a moved widget, although AccessKit nodes
carry parent-relative transforms and descendants are unchanged; panning the Map therefore resent
every mounted card's subtree each frame. Paint is still requested for the whole subtree. The fix is
the `compose-local-accessibility` commit.
`compose_accessibility` covers the emitted node set and consumer-computed descendant bounds; the
Map panning regression bounds nodes per local pan frame. Platform adapters must derive descendant
geometry from ancestor transforms, as the AT-SPI exposure regressions check.

Recite also carried a commit that formatted the parent widget name only when a child lookup
panics. Upstream made the same change (linebender/xilem#1858), so the rebase dropped it.

Windows can also render with Vello CPU. `imaging_vello_cpu` rasterises each frame on the host and
uploads it into the target texture, so presentation only copies pixels. `vello_cpu_fallback`
builds both backends and picks one per adapter: Vello CPU when wgpu reports a CPU adapter such as
llvmpipe or WARP, GPU Vello otherwise. `MASONRY_IMAGING_BACKEND=vello|vello_cpu` overrides the
choice, and the selection is logged once at `info`. `vello_cpu_multithreading` enables Vello CPU's
worker pool. On a 4-core host without a GPU, Recite Writer's edit-to-paint p50 fell from 26 ms
(GPU Vello on llvmpipe) to 13 ms and frame pacing from about 35 to 60 fps (plethu/recite#283).
Remove this when Masonry selects a CPU renderer for software adapters itself.

`softbuffer_fallback` presents Vello CPU frames through `softbuffer` instead of wgpu when every
wgpu adapter is a CPU implementation such as llvmpipe or WARP, and when wgpu cannot create a surface
at all, for example on a host with no Vulkan, Metal, D3D or GL driver, where Masonry previously
panicked. On a CPU-only adapter the wgpu path only adds an emulated GPU copy: on the same 4-core
host, Writer typed with 0.66 cores of CPU through `softbuffer` against 1.1-1.2 through llvmpipe, at
the same 60 fps and edit-to-paint (plethu/recite#283). The choice is logged with
`presenter="softbuffer"`, and `MASONRY_IMAGING_BACKEND=softbuffer|vello_cpu|vello` overrides it.
Remove this when Masonry presents CPU frames without wgpu itself.


Masonry's paint pass also reports damage regions (linebender/xilem#789): `VisualLayerPlan::damage`
lists the window rectangles that changed since the previous frame, from each widget's recorded draw
bounds rather than its layout box, so it covers moved, resized, stashed, added, removed and
reordered widgets, scrolled viewports and layers, and diffs a repainted widget's old and new scenes
so only changed draws count. It reports full damage on the first frame, on resize, scale factor and
default property changes, around debug and inspector overlays, and past 60% of the window. The
softbuffer presenter keeps a persistent framebuffer, re-rasterises only damaged rectangles widened
to Vello CPU's 256×4 wide tiles with off-region draws culled. It presents only the damage on
Wayland, whose compositor keeps the window's buffer. X11 and Win32 can discard window contents, and
winit merges their redraw requests with Masonry's, so there every present covers the whole window,
as do presents after unrequested redraws, resizes, scale changes and the window becoming visible. The wgpu presenters, including Vello CPU through
wgpu, still render in full. Geometry and text match a full render exactly and image and gradient
pixels within one unit. In Writer's gallery harness at 1200×800 with desk grain, typing into a
Script draft re-rasterised about 5% of the window per keystroke, and rasterising fell from a 12 ms
p50 to 3.4 ms with identical pixels; damage tracking added about 0.15 ms per redraw with Masonry
optimised and about 1 ms unoptimised. Remove this when Masonry reports damage and a host presents
it upstream.

Xilem's `text_input` and `prose` views take a `line_height` builder, as `label` does. The view sets
it on the inner `TextArea` when it builds the widget, so the first frame already uses it, and
reinserts it on rebuild when the value changes. The default is `theme::TEXT_LINE_HEIGHT`, the value
`default_text_styles` already applied. Writer sets Literata at 1.45 on its multi-line manuscript
inputs. Remove this when upstream views expose line height.

`Portal` takes an optional accessible label, written to its `ScrollView` node, and a `focusable`
flag with `set_focusable`; xilem's `portal` view exposes both. A non-focusable portal leaves the Tab
order, while unhandled key events from focused descendants still bubble to its keyboard scrolling.
Masonry caches `accepts_focus` when a widget is added, so the commit adds an
`accepts_focus_changed` context method that re-reads it in the next focusable pass; the constrain
setters call it as well. Portal viewport changes also request an accessibility update: since the
`compose-local-accessibility` change, compose no longer rebuilds the portal's own node, so its
`scroll_y` went stale (`portal_accessibility_node_exposes_scroll` failed). A portal that loses
focusability while focused keeps focus until it moves. Remove this when upstream portals offer both.

`Button` takes a `ButtonAccessState` (`with_access_state`, `set_access_state`) whose `expanded`,
`selected` and `toggled` values are written to its AccessKit node; xilem's `button` view adds
`expanded`, `selected` and `toggled` builders. Writer uses them instead of "▸/▾" in disclosure
names, for selected picker and list rows, and for toggles such as "Pin reference". A `toggled` value
makes AT-SPI expose a toggle button with a pressed state. `DisclosureButton` reports its disclosed
state as `expanded` and advertises `Click`. Remove this when upstream buttons report these states.

`Button` also takes a role override (`with_role`, `set_role`), an explicit accessible label
(`with_accessible_label`, `set_accessible_label`) and a `focusable` flag (`with_focusable`,
`set_focusable`); xilem's `button` view adds `role`, `accessible_label` and `focusable`. Writer
builds roving-focus tab lists from them: `Role::Tab` buttons with `selected`, inside its own
`TabList` group, where only the selected tab is focusable and Writer handles the arrow keys. A
non-focusable button behaves like `tabindex="-1"`: Tab skips it, but pointer presses,
`request_focus`, `set_focus` and the AccessKit `Focus` action still focus it, since Masonry's focus
pass never checks `accepts_focus`, and it still activates. AccessKit names `Button` roles from their
descendant labels but not `Tab`, so tabs need the explicit label. AT-SPI exposes such a node as a
`page tab` with `selectable` and `selected` states. Remove this when upstream buttons support tabs.

`RadioButton` takes an explicit accessible label (`with_accessible_label`, `set_accessible_label`),
and xilem's `radio_button` view adds `accessible_label`. Writer's settings segments show a short face
("Left | Right") while each radio keeps a name that makes sense out of context ("Script pane on the
left"). Remove this when upstream radio buttons accept a name separate from their label.

`VirtualScroll` takes the same accessible label and focusable flag as `Portal`
(`with_accessible_label`, `set_accessible_label`, `with_focusable`, `set_focusable`), and xilem's
`virtual_scroll` view adds `accessible_label` and `focusable`. A non-focusable list leaves the Tab
order while keys its focused items leave unhandled still bubble to its scrolling, and scrolling now
requests an accessibility update so the node's scroll position and actions stay current. Writer's
picker results list uses it. Remove this when upstream virtual scroll offers both.

`FlexBasis::Length` gives a flex child a fixed basis that it gets before free space is shared, so
a flexible child keeps that minimum while the row has room for every basis. Writer's file tab uses
it to stay at least 96px wide and grow from there. Remove this when upstream `Flex` supports
minimum sizes for flexible children.
