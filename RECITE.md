# Recite Writer patches

This branch carries fixes that [Recite](https://github.com/plethu/recite)'s Writer needs in Masonry,
on top of Xilem revision `271a27a6d4a930f7878d404f9014e3c50a3a9b88`. Each fix is its own commit,
so it can be proposed upstream separately. None of them is upstream yet. Recite depends on this
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
`ShapedText` widget now provides both (plethu/recite#288), so those commits are reverted on this
branch.

Masonry's compose pass additionally rebuilds an accessibility node only when that widget's own
transform changes. Upstream marked every descendant of a moved widget, although AccessKit nodes
carry parent-relative transforms and descendants are unchanged; panning the Map therefore resent
every mounted card's subtree each frame. Paint is still requested for the whole subtree. The fix is
the `compose-local-accessibility` commit.
`compose_accessibility` covers the emitted node set and consumer-computed descendant bounds; the
Map panning regression bounds nodes per local pan frame. Platform adapters must derive descendant
geometry from ancestor transforms, as the AT-SPI exposure regressions check.

Every Masonry pass also named each parent widget before recursing into its children, only to use
that name in an unreachable panic message. That split each widget type name on every frame. The name
is now formatted only when the panic fires; see
the `recurse-lazy-name` commit.
