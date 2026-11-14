# Raspberry Flame asset scope

On 2026-11-14 the owner selected the original Raspberry Flame badge.
Its master PNG/SVG files are in `assets/branding/`.

The laptop badge was replaced locally in 112 runtime frames across 14
animations: seven standing emotions/laptop states, floor_focus_laptop,
and left/right wall focus_laptop, question and sadness. The other 192
runtime frames were byte-identical. Per-image binary masks verified exact
alpha and unchanged pixels outside the badge repair. Accepted source/raw
sheets were repaired in the private working archive too.

The README hero was regenerated and retains 24 frames, 212 by 448 pixels,
and original timing. Plugin and desktop packages copy the current 304
runtime frames from the manifest. The owner reviewed the design; the
full replacement received a visual and mechanical review.

The public snapshot excludes older artwork, source/raw generation sheets,
rejected variants, moodboard references, private captures, intermediate
proofs and previous Git history. Those remain in the private archive.
The local repair masks and baseline-backed editing helper depend on that
archive and are therefore not shipped as a reproducible public pipeline.
The public packaging and README-GIF builders work from the shipped PNGs.

This replacement does not claim legal clearance of excluded historical
material. See LICENSE_AUDIT.md for the scope of the artwork grant.
