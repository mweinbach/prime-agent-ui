# Local GPUI patches

This directory is the crates.io `gpui` 0.2.2 package (examples removed), wired in
through `[patch.crates-io]` in the workspace `Cargo.toml`. The first commit that added
it is an unmodified copy, so `git diff <that commit> -- vendor/gpui` shows every local
change.

The changes backport macOS text rendering fixes that landed in upstream Zed after the
0.2.2 release (`crates/gpui_apple/src/text_system.rs` and `crates/gpui/src/window.rs`
upstream). Drop this directory once a published GPUI release includes them.

## macOS font smoothing (glyph dilation)

AppKit text is drawn with CoreGraphics font smoothing, which thickens strokes by an
amount chosen from the foreground luminance. GPUI 0.2.2 rasterized every glyph into an
alpha mask with smoothing off, so text looked thinner and lighter than in native apps,
especially light text on dark backgrounds.

- `RenderGlyphParams` has a `dilation` level (0–4) that is part of the glyph cache key.
- `PlatformTextSystem::glyph_dilation_for_color` defaults to 0. The macOS text system
  returns the CoreGraphics level for the text color unless the user set
  `AppleFontSmoothing` to `0`.
- The macOS rasterizer turns on `CGContextSetShouldSmoothFonts` and uses a gray fill of
  the same luminance so CoreGraphics picks the matching stroke weight. The mask stays
  alpha-only; the real color is still applied by the sprite shader.

## Unclipped antialiasing

The macOS rasterizer pads font-kit's outline bounds by one device pixel on each side, so
antialiased and dilated edge pixels are no longer cut off the glyph mask.

## Pixel-aligned baselines

- `SUBPIXEL_VARIANTS_Y` is 1 on every platform. On macOS it was 4, so a line's sharpness
  depended on its fractional y offset. Baselines now snap to whole device pixels like AppKit.
- `quantize_glyph_origin` snaps glyph origins to the nearest quarter pixel horizontally
  and whole pixel vertically. Before, 0.2.2 always rounded down, which nudged every glyph
  up and to the left.

Linux and Windows rasterization is unchanged apart from the nearest-step rounding.
