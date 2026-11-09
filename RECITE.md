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
