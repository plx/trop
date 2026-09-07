# Harbor alignment experiment

The approved aligned pair is in `../../assets/harbor-aligned/`, both **1642 × 958**.
The daytime PNG is byte-identical to the original. Night is a deterministic
rendering of that daytime drawing with separate illumination. The existing
production heroes and their site references have not been replaced.

Open [review.html](review.html) for synchronized theme transitions, a same-theme
wipe, native-pixel inspection, and downloads. The static
[before/after board](before-after.png) and [detail crops](detail-comparison.png)
show the remaining artistic difference: the candidate has somewhat different
night engraving contrast, especially in the crane and foreground. The user
approved the visual result on 2026-09-07 and requested that it be committed.

## What was tried

One built-in ImageGen edit used the original day as the geometry target and
original night as the appearance reference. The exact
[prompt](imagegen-prompt.txt) asked for registered relighting. It returned the
correct resolution and similar atmosphere but redrew small fixed details.
That full-frame output was rejected as the final night image. It remains in
the workspace’s ignored `.context/hero-alignment/night-generated-v1.png`.

The final construction uses:

1. Original daytime grayscale at its native pixel grid for every fixed detail.
2. Separate tone mappings for exposed engraving and opaque surfaces, with
   master-coordinate surface masks. Broad illumination and local contrast
   statistics come from the original night; its stationary linework is not
   copied into the output.
3. Sparse stars extracted from feature-free ImageGen sky, retained losslessly
   as float32 values in `generated-stars.npz`.
4. Increased emission on the master crescent, small lamp cores and halos at
   master fixture coordinates, city lights constrained to master shoreline
   detail, and warm reflection envelopes modulated by master water marks.
5. No resizing, warping, or subsequent full-frame generation.

Global inversion made solid objects too luminous. Local covariance transfer
between the unregistered originals erased fine lines. The retained recipe
uses master detail plus smooth tone/contrast fields instead. Subjective
fidelity remains a separate check from geometric registration.

## Replay

Dependencies used: Python 3.12, NumPy 2.5.3, Pillow 12.3.0, and
opencv-python-headless 5.0.0.93. No API key or further ImageGen call is needed.

From the repository root:

```sh
uv venv .context/harbor-replay-venv
uv pip install --python .context/harbor-replay-venv/bin/python \
  numpy==2.5.3 pillow==12.3.0 opencv-python-headless==5.0.0.93
.context/harbor-replay-venv/bin/python \
  trop-design-system/experiments/harbor-alignment/render.py \
  --output .context/harbor-replay
```

The output directory must be new or empty. The renderer writes the pair plus
diagnostic masks and lighting layers. It checks source hashes and refuses
different originals; provide `--day` and `--night` paths if the production
assets have since changed. Its palette, polygons, and detection thresholds
are specific to this scene.

Input SHA-256:

- Day: `2cf9ff3b0e80214f3ea9f602794eff9fef38b895773c22d7a941c23d3eb82b9c`
- Night: `b84c90aa724af042a481ede4020ec91d8079e643233d4a9473b9f690821cac5f`

Candidate night SHA-256:
`7e5799393f0cdc7d959d75fd73f498c0db1d1bfc731f8f171e0ce4f60e257834`

A fresh replay produced byte-identical PNGs for both members of the pair.

## Alignment evidence

`regions.json` specifies 11 stable patches: crane apex, cables, beam, ship
bow, containers, buoy, bollard face/rim, rope, quay stones, and shoreline.

```sh
.context/harbor-replay-venv/bin/python \
  trop-design-system/experiments/harbor-alignment/check_alignment.py \
  --master trop-design-system/assets/harbor-hero-light.png \
  --candidate trop-design-system/assets/harbor-aligned/harbor-hero-dark.png \
  --regions trop-design-system/experiments/harbor-alignment/regions.json \
  --output .context/harbor-alignment-check.json
```

The check searches integer offsets ±6 px using absolute normalized grayscale
high-pass correlation, allowing the line polarity to reverse between themes.
All **11/11 candidate patches** peak at **(0, 0)**, versus 1/11 in the original
pair. See `alignment-before.json`, `alignment-after.json`, and the combined
`alignment-metrics.json` for confidence and search-boundary flags. Some
original matches are weak, so their offsets are approximate diagnostic
findings, not precise movement measurements.

This is sampled evidence, not exhaustive proof of every contour. Structural
provenance comes from retaining one drawing; mask boundaries, illumination,
and any apparent contour changes still require visual review. The helper was
also checked against a known translated image with inverted polarity, an
identity pair, empty-detail patches, and mismatched dimensions.

## Reusable skill

The personal `$aligned-theme-images` skill was installed at
`~/.codex/skills/aligned-theme-images/`. It captures the shared-master workflow,
the engraving technique and its limits, and the alignment helper. It does not
treat this scene’s masks, colors, or artistic approval as universal defaults.

## Validation

- Fresh replay: both output PNGs byte-identical to the saved candidates.
- Alignment helper: 11/11 zero-offset candidate samples, plus synthetic
  translation/polarity and invalid-input checks.
- Skill structure validator passed.
- Interactive comparison reviewed in Chrome, including the wipe and transition
  controls; the comparison tab was left open for review.
- Site formatting, design-system boundary check, Astro diagnostics, spelling,
  and production build passed. The 45 responsive/accessibility browser tests
  passed with `TROP_SITE_TEST_PORT=44371 npm test`. The initial default-port
  run timed out because another workspace occupied port 4321.

The site tests cover the existing production integration; the separately
saved candidate artwork was reviewed in the comparison page.
