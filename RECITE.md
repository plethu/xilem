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

